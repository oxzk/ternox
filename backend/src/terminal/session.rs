//! 终端 Session：状态、生命周期与共享资源管理。
//!
//! portable-pty 的 Master / Writer / Child 并非都是 Sync，因此按其线程安全
//! 约束拆分为独立的 `Mutex<Option<..>>`，而不是塞进一个跨线程共享的大结构。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use portable_pty::{ChildKiller, MasterPty, PtySize};
use serde::Serialize;
use tokio::sync::{oneshot, watch};
use uuid::Uuid;

use super::buffer::OutputBuffer;
use crate::config::{MAX_COLS, MAX_ROWS};

/// 终端生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifecycle {
    Running,
    Detached,
    Exited,
    Closed,
}

impl Lifecycle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Detached => "detached",
            Self::Exited => "exited",
            Self::Closed => "closed",
        }
    }

    /// 是否允许 WebSocket 接入。
    pub fn connectable(self) -> bool {
        matches!(self, Self::Running | Self::Detached)
    }
}

/// 终端信息（API 返回）。
#[derive(Debug, Serialize)]
pub struct TerminalInfo {
    pub id: String,
    pub status: &'static str,
    pub cols: u16,
    pub rows: u16,
}

/// 新连接的持有凭证。
pub struct AttachHandle {
    /// 连接代号：用于断开时校验自己仍是活跃连接（防止旧连接误清理新连接）。
    pub generation: u64,
    /// 新连接建立时被触发，旧连接据此自行退出。
    pub cancel: oneshot::Receiver<()>,
}

struct State {
    lifecycle: Lifecycle,
    cols: u16,
    rows: u16,
    created_at: Instant,
    detached_at: Option<Instant>,
    exited_at: Option<Instant>,
    exit_code: Option<i32>,
    active_conn: Option<(u64, oneshot::Sender<()>)>,
}

/// 从状态中取走活跃连接的 sender（attach / kick / shutdown 统一走这里踢人）。
fn take_active_conn(state: &mut State) -> Option<oneshot::Sender<()>> {
    state.active_conn.take().map(|(_, sender)| sender)
}

pub struct TerminalSession {
    /// 终端标识（日志与 API 返回使用）。
    pub(crate) id: Uuid,
    /// 有界输出缓冲（带序列号）。写入统一走 [`append_output`]。
    buffer: OutputBuffer,
    /// 输出/退出事件信号（不携带载荷，读到信号后调 [`replay_output`]）。
    notify: watch::Sender<()>,
    /// Shell 输入通道（交给专用阻塞写线程）。
    input: Mutex<Option<std::sync::mpsc::SyncSender<Vec<u8>>>>,
    /// PTY 主端：持有它 PTY 才存活；drop 后读线程随之退出。
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    /// Child 的 killer 句柄，可独立于 wait 线程发送终止信号。
    killer: Mutex<Option<Box<dyn ChildKiller + Send + Sync>>>,
    /// Shell 进程组 id（portable-pty 将子进程设为会话首进程）。
    pgid: Option<u32>,
    state: Mutex<State>,
    next_conn_generation: AtomicU64,
}

impl TerminalSession {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: Uuid,
        buffer: OutputBuffer,
        notify: watch::Sender<()>,
        input: std::sync::mpsc::SyncSender<Vec<u8>>,
        killer: Box<dyn ChildKiller + Send + Sync>,
        master: Box<dyn MasterPty + Send>,
        pgid: Option<u32>,
        cols: u16,
        rows: u16,
    ) -> Self {
        let now = Instant::now();
        Self {
            id,
            buffer,
            notify,
            input: Mutex::new(Some(input)),
            master: Mutex::new(Some(master)),
            killer: Mutex::new(Some(killer)),
            pgid,
            state: Mutex::new(State {
                lifecycle: Lifecycle::Running,
                cols,
                rows,
                created_at: now,
                // 创建即视为空闲起点：无人连接也会被空闲回收
                detached_at: Some(now),
                exited_at: None,
                exit_code: None,
                active_conn: None,
            }),
            next_conn_generation: AtomicU64::new(1),
        }
    }

    pub fn lifecycle(&self) -> Lifecycle {
        self.state.lock().unwrap().lifecycle
    }

    pub fn info(&self) -> TerminalInfo {
        let state = self.state.lock().unwrap();
        TerminalInfo {
            id: self.id.to_string(),
            status: state.lifecycle.as_str(),
            cols: state.cols,
            rows: state.rows,
        }
    }

    /// 注册新的活跃连接：踢掉旧连接，返回本次连接的凭证；
    /// 终端已关闭或 Shell 已退出时拒绝。
    pub fn attach(&self) -> Option<AttachHandle> {
        let mut state = self.state.lock().unwrap();
        if !state.lifecycle.connectable() {
            return None;
        }
        let generation = self.next_conn_generation.fetch_add(1, Ordering::Relaxed);
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        if let Some(old) = take_active_conn(&mut state) {
            let _ = old.send(());
        }
        state.active_conn = Some((generation, cancel_tx));
        if state.lifecycle == Lifecycle::Detached {
            state.lifecycle = Lifecycle::Running;
        }
        state.detached_at = None;
        Some(AttachHandle {
            generation,
            cancel: cancel_rx,
        })
    }

    /// 连接断开：仅当自己仍是活跃连接时生效。
    pub fn detach(&self, generation: u64) {
        let mut state = self.state.lock().unwrap();
        match &state.active_conn {
            Some((current, _)) if *current == generation => {}
            _ => return,
        }
        state.active_conn = None;
        if state.lifecycle == Lifecycle::Running {
            state.lifecycle = Lifecycle::Detached;
            state.detached_at = Some(Instant::now());
        }
    }

    /// 主动踢掉当前活跃连接（应用关闭时调用）。
    pub fn kick(&self) {
        let mut state = self.state.lock().unwrap();
        if let Some(sender) = take_active_conn(&mut state) {
            let _ = sender.send(());
        }
    }

    /// 无活跃连接的起始时间，用于空闲回收判定。
    pub fn idle_since(&self) -> Option<Instant> {
        let state = self.state.lock().unwrap();
        if state.active_conn.is_some() {
            return None;
        }
        match state.lifecycle {
            Lifecycle::Running | Lifecycle::Detached => {
                Some(state.detached_at.unwrap_or(state.created_at))
            }
            Lifecycle::Exited => Some(state.exited_at.unwrap_or(state.created_at)),
            Lifecycle::Closed => None,
        }
    }

    /// 唯一的输出写入入口：追加缓冲并唤醒活跃 WebSocket。
    pub fn append_output(&self, data: &[u8]) -> u64 {
        let sequence = self.buffer.append(data);
        self.notify.send_replace(());
        sequence
    }

    /// 订阅输出变更信号（收到信号后调用 [`replay_output`] 拉取增量）。
    pub fn subscribe_output(&self) -> watch::Receiver<()> {
        self.notify.subscribe()
    }

    /// 回放 `sequence > last_sequence` 的输出；客户端位置已被淘汰时返回空洞。
    pub fn replay_output(&self, last_sequence: u64) -> super::buffer::Replay {
        self.buffer.replay_after(last_sequence)
    }

    /// 最新输出序列号；缓冲为空时为 0。
    pub fn latest_sequence(&self) -> u64 {
        self.buffer.latest()
    }

    /// 调整终端尺寸（校验合法范围后作用于 PTY）。
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        if !(2..=MAX_COLS).contains(&cols) || !(2..=MAX_ROWS).contains(&rows) {
            return Err(format!("终端尺寸超出允许范围 (2..={MAX_COLS} x 2..={MAX_ROWS})"));
        }
        let mut state = self.state.lock().unwrap();
        if state.cols == cols && state.rows == rows {
            return Ok(());
        }
        let master = self.master.lock().unwrap();
        let Some(master) = master.as_ref() else {
            return Err("terminal closed".into());
        };
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("PTY resize failed: {e}"))?;
        state.cols = cols;
        state.rows = rows;
        Ok(())
    }

    /// 转发键盘输入到 PTY 写线程；通道满则丢弃（流量控制）。
    pub fn send_input(&self, data: Vec<u8>) -> Result<(), &'static str> {
        if data.is_empty() {
            return Ok(());
        }
        let input = self.input.lock().unwrap();
        match input.as_ref() {
            Some(tx) => tx.try_send(data).map_err(|_| "pty input unavailable"),
            None => Err("terminal closed"),
        }
    }

    /// Shell 退出回调（由子进程等待线程触发）。
    pub fn mark_exited(&self, exit_code: i32) {
        {
            let mut state = self.state.lock().unwrap();
            if state.lifecycle == Lifecycle::Closed {
                return;
            }
            state.lifecycle = Lifecycle::Exited;
            state.exit_code = Some(exit_code);
            state.exited_at = Some(Instant::now());
        }
        // 唤醒 WebSocket 写任务以发送 exit 消息
        self.notify.send_replace(());
    }

    /// Shell 已退出时返回退出码（供写任务发送 exit 消息）。
    pub fn exit_code_if_exited(&self) -> Option<i32> {
        let state = self.state.lock().unwrap();
        if state.lifecycle == Lifecycle::Exited {
            state.exit_code
        } else {
            None
        }
    }

    /// 向 Shell 进程组发送 SIGTERM。
    pub fn terminate(&self) {
        #[cfg(unix)]
        if let Some(pgid) = self.pgid {
            let sent = unsafe { libc::killpg(pgid as libc::pid_t, libc::SIGTERM) };
            if sent == 0 {
                tracing::debug!(terminal_id = %self.id, pgid, "已向进程组发送 SIGTERM");
                return;
            }
        }
        self.kill();
    }

    /// 关闭终端：释放 master（SIGHUP 传播给前台作业）→ SIGTERM → 等待 → SIGKILL → 释放资源。
    pub async fn shutdown(&self) {
        let was_exited = {
            let mut state = self.state.lock().unwrap();
            if state.lifecycle == Lifecycle::Closed {
                return;
            }
            let was_exited = state.lifecycle == Lifecycle::Exited;
            state.lifecycle = Lifecycle::Closed;
            if let Some(sender) = take_active_conn(&mut state) {
                let _ = sender.send(());
            }
            was_exited
        };
        // 先释放 PTY 主端：内核向 session leader 发 SIGHUP，bash 会转发给前台作业
        {
            let _ = self.master.lock().unwrap().take();
        }
        if !was_exited {
            // Shell 已退出的终端不再对旧 pgid 发信号，避免 PID 复用误杀无关进程组
            self.terminate();
            tokio::time::sleep(Duration::from_secs(2)).await;
            self.kill();
        }
        self.release_resources();
    }

    /// 释放剩余资源：killer 与输入通道关闭后写线程退出。
    fn release_resources(&self) {
        let _ = self.killer.lock().unwrap().take();
        let _ = self.input.lock().unwrap().take();
        self.buffer.clear();
        self.notify.send_replace(());
    }

    fn kill(&self) {
        if let Some(killer) = self.killer.lock().unwrap().as_mut() {
            let _ = killer.kill();
        }
    }
}
