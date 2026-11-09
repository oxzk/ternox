//! PTY 与 Shell 进程创建。
//!
//! PTY Reader / Writer 是阻塞式 I/O：Reader 用独立阻塞线程持续读取，
//! Writer 用专用阻塞线程消费输入通道；Tokio 侧只通过 channel 与其交互。

use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use portable_pty::{CommandBuilder, PtySize};
use tokio::sync::watch;
use uuid::Uuid;

use super::buffer::OutputBuffer;
use super::session::TerminalSession;
use crate::config::{TERMINAL_BUFFER_SIZE, TERMINAL_SHELL};

/// PTY 输出读取块大小。
const READ_CHUNK_SIZE: usize = 8192;
/// PTY 输入通道容量（流量控制：写满即丢弃输入）。
const INPUT_CHANNEL_CAPACITY: usize = 128;
/// 初始终端尺寸。
pub const DEFAULT_COLS: u16 = 120;
pub const DEFAULT_ROWS: u16 = 30;

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("工作目录不可用: {0}")]
    Cwd(String),
    #[error("PTY 创建失败: {0}")]
    Pty(String),
}

/// 创建唯一终端：PTY Pair + Shell 进程 + 读/写/等待线程。
///
/// 阻塞操作（openpty、spawn）应由调用方放在 `spawn_blocking` 中执行。
pub fn create_session(
    initial_cols: u16,
    initial_rows: u16,
) -> Result<Arc<TerminalSession>, PtyError> {
    let cwd = resolve_cwd()?;
    let shell = resolve_shell(TERMINAL_SHELL);

    let pair = portable_pty::native_pty_system()
        .openpty(PtySize {
            rows: initial_rows,
            cols: initial_cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| PtyError::Pty(e.to_string()))?;

    let mut command = CommandBuilder::new(&shell);
    command.cwd(&cwd);
    // 环境默认继承父进程（HOME 等由容器提供），此处补充 TERM
    command.env("TERM", "xterm-256color");
    // 从 zsh 启动时父进程可能导出 zsh 语法的 PS1/PROMPT（如 oh-my-zsh 主题），
    // bash 会原样打印这些 %[..% 序列；PROMPT_COMMAND/ENV/BASH_ENV 则会被
    // bash 直接执行，属于注入面，一并清除
    for var in [
        "PS1", "PS2", "PS3", "PS4", "PROMPT", "RPROMPT", "RPS1",
        "PROMPT_COMMAND", "ENV", "BASH_ENV",
    ] {
        command.env_remove(var);
    }
    // 认证凭据不能泄漏给用户可操作的 shell（进程环境会被子进程继承）
    command.env_remove("AUTH_USERNAME");
    command.env_remove("AUTH_PASSWORD");
    // 无 bashrc 的环境下给出可读的默认提示符（用户 bashrc 设置的 PS1 仍会覆盖它）
    command.env("PS1", "\\u@\\h:\\w\\$ ");

    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|e| PtyError::Pty(e.to_string()))?;
    // spawn 成功后的任何失败路径都必须杀掉子进程，避免遗留孤儿 Shell
    let reader = match pair.master.try_clone_reader() {
        Ok(reader) => reader,
        Err(e) => {
            let _ = child.kill();
            return Err(PtyError::Pty(e.to_string()));
        }
    };
    let writer = match pair.master.take_writer() {
        Ok(writer) => writer,
        Err(e) => {
            let _ = child.kill();
            return Err(PtyError::Pty(e.to_string()));
        }
    };
    let killer = child.clone_killer();
    // child 移交等待线程后仍需兜底清理能力
    let mut cleanup_killer = child.clone_killer();
    // process_group_leader 返回 pid_t(i32)，统一转成 u32
    let pgid = pair
        .master
        .process_group_leader()
        .map(|p| p as u32)
        .or(child.process_id());
    // 子进程已持有 slave 的 fd，主进程侧用完即弃
    drop(pair.slave);

    let (notify, _) = watch::channel(());
    let buffer = OutputBuffer::new(TERMINAL_BUFFER_SIZE);
    let (input_tx, input_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(INPUT_CHANNEL_CAPACITY);
    let (exit_tx, mut exit_rx) = tokio::sync::mpsc::unbounded_channel::<i32>();

    let session = Arc::new(TerminalSession::new(
        Uuid::new_v4(),
        buffer,
        notify,
        input_tx,
        killer,
        pair.master,
        pgid,
        initial_cols,
        initial_rows,
    ));

    // 输入线程：从通道取输入，阻塞写入 PTY（通道关闭或写失败时退出）
    if let Err(e) = std::thread::Builder::new()
        .name("ternox-pty-writer".into())
        .spawn(move || {
            let mut writer = writer;
            for chunk in input_rx {
                if writer.write_all(&chunk).and_then(|_| writer.flush()).is_err() {
                    break;
                }
            }
        })
    {
        let _ = cleanup_killer.kill();
        return Err(PtyError::Pty(format!("启动输入线程失败: {e}")));
    }

    // 输出读取线程：阻塞读 PTY → 追加缓冲 → 通知活跃 WebSocket
    {
        let session = session.clone();
        if let Err(e) = std::thread::Builder::new()
            .name("ternox-pty-reader".into())
            .spawn(move || {
                let mut reader = reader;
                let mut buf = [0u8; READ_CHUNK_SIZE];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            session.append_output(&buf[..n]);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            })
        {
            let _ = cleanup_killer.kill();
            return Err(PtyError::Pty(format!("启动输出线程失败: {e}")));
        }
    }

    // 子进程等待线程：阻塞 wait，退出码经通道回传给异步任务
    {
        let runtime = tokio::runtime::Handle::current();
        let session_for_exit = session.clone();
        if let Err(e) = std::thread::Builder::new()
            .name("ternox-pty-wait".into())
            .spawn(move || {
                let code = match child.wait() {
                    Ok(status) if status.success() => 0,
                    // portable-pty 已归一化退出状态：信号死亡时 exit_code 退化为 1
                    Ok(status) => (status.exit_code() as i64).clamp(1, 255) as i32,
                    Err(_) => -1,
                };
                let _ = exit_tx.send(code);
            })
        {
            let _ = cleanup_killer.kill();
            return Err(PtyError::Pty(format!("启动等待线程失败: {e}")));
        }
        runtime.spawn(async move {
            while let Some(code) = exit_rx.recv().await {
                session_for_exit.mark_exited(code);
            }
        });
    }

    tracing::debug!(
        terminal_id = %session.id,
        shell = %shell,
        cwd = %cwd.display(),
        pgid = ?pgid,
        "终端已创建"
    );
    Ok(session)
}

/// 解析 Shell 初始工作目录：用户 `$HOME`（不做路径白名单限制）。
fn resolve_cwd() -> Result<PathBuf, PtyError> {
    let home = std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| PtyError::Cwd("HOME 未设置，无法确定工作目录".into()))?;
    let dir = PathBuf::from(&home);
    if !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|e| PtyError::Cwd(format!("{} 创建失败: {e}", dir.display())))?;
    }
    std::fs::canonicalize(&dir)
        .map_err(|e| PtyError::Cwd(format!("{} 解析失败: {e}", dir.display())))
}

/// 解析 Shell 路径：配置值 → /bin/bash → /bin/sh 兜底。
fn resolve_shell(configured: &str) -> String {
    for candidate in [configured, "/bin/bash", "/bin/sh"] {
        if Path::new(candidate).exists() {
            return candidate.to_string();
        }
    }
    tracing::warn!(configured, "配置的 Shell 与常见回退均不存在，交给 spawn 报错");
    configured.to_string()
}
