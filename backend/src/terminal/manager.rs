//! 唯一终端会话的管理：创建、获取、空闲回收与全局关闭。

use std::sync::Arc;
use std::time::Duration;

use tokio::time::{interval, MissedTickBehavior};

use super::pty::{create_session, PtyError, DEFAULT_COLS, DEFAULT_ROWS};
use super::session::{Lifecycle, TerminalSession};
use crate::config::TERMINAL_IDLE_TIMEOUT;

/// 回收器检查间隔。
const REAPER_INTERVAL_SECS: u64 = 15;
/// 已退出终端的保留时间（保留缓冲供前端查看退出信息）。
const EXITED_KEEP_SECS: u64 = 300;

#[derive(Clone)]
pub struct TerminalManager {
    session: Arc<tokio::sync::Mutex<Option<Arc<TerminalSession>>>>,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            session: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    /// 获取当前终端（可能为 None，也可能已退出/关闭）。
    pub async fn current(&self) -> Option<Arc<TerminalSession>> {
        self.session.lock().await.clone()
    }

    /// 获取或初始化唯一终端。
    ///
    /// 终端尚未创建或已退出/关闭时创建新的 PTY 和 Shell；
    /// 其余情况返回现有终端。创建串行化，避免并发重复创建。
    pub async fn get_or_create(&self) -> Result<Arc<TerminalSession>, PtyError> {
        let mut guard = self.session.lock().await;
        if let Some(existing) = guard.as_ref() {
            if existing.lifecycle().connectable() {
                return Ok(existing.clone());
            }
        }
        let session =
            tokio::task::spawn_blocking(|| create_session(DEFAULT_COLS, DEFAULT_ROWS))
                .await
                .map_err(|e| PtyError::Pty(format!("终端创建任务失败: {e}")))??;
        *guard = Some(session.clone());
        Ok(session)
    }

    /// 空闲回收后台任务：
    /// - 断开（或从未连接）超过 TERMINAL_IDLE_TIMEOUT 的终端 → 清理；
    /// - 已退出的终端保留一段时间后移除。
    pub async fn run_reaper(&self) {
        let mut ticker = interval(Duration::from_secs(REAPER_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let taken = {
                let mut guard = self.session.lock().await;
                match guard.as_ref() {
                    Some(session) => {
                        let keep = match session.lifecycle() {
                            Lifecycle::Exited => Duration::from_secs(EXITED_KEEP_SECS),
                            _ => TERMINAL_IDLE_TIMEOUT,
                        };
                        match session.idle_since() {
                            Some(since) if since.elapsed() >= keep => guard.take(),
                            _ => None,
                        }
                    }
                    None => None,
                }
            };
            if let Some(session) = taken {
                session.shutdown().await;
            }
        }
    }

    /// 通知活跃 WebSocket 关闭（优雅关闭第一步，让 axum 尽快结束连接）。
    pub async fn kick_connections(&self) {
        if let Some(session) = self.session.lock().await.as_ref() {
            session.kick();
        }
    }

    /// 应用关闭：终止 Shell 进程组并释放资源。
    pub async fn shutdown_all(&self) {
        let taken = self.session.lock().await.take();
        if let Some(session) = taken {
            session.shutdown().await;
        }
    }
}
