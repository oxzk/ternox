//! WebSocket 终端协议引擎：分帧、断线恢复、输入转发与会话复验。
//!
//! 协议（与 frontend/src/services/protocol.ts 保持同步，改动必须双端同时）：
//! - 二进制帧：`0x01 + 输入`（浏览器→Shell），`0x02 + u64 序列号 + 输出`（Shell→浏览器）。
//! - 文本帧：JSON 控制消息（connected/resize/resume/ping/pong/exit/error）。
//! - 关闭码 4001：连接被新连接替换。

use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::buffer::Replay;
use super::session::TerminalSession;
use crate::auth::Auth;
use crate::config::TERMINAL_PONG_TIMEOUT;

/// 二进制消息类型：浏览器 → Shell 输入。
const INPUT_TYPE: u8 = 0x01;
/// 二进制消息类型：Shell → 浏览器输出。
const OUTPUT_TYPE: u8 = 0x02;
/// 序列号字节数（大端 u64）。
const SEQUENCE_LEN: usize = 8;
/// 自定义 WebSocket 关闭码：连接被新连接替换（多窗口互踢时旧窗口据此停止重连）。
const CLOSE_REPLACED: u16 = 4001;
/// 会话复验周期（登出/轮换/过期断开 + 心跳判死共用，需小于 TERMINAL_PONG_TIMEOUT）。
const SESSION_REVALIDATE_INTERVAL: Duration = Duration::from_secs(30);

/// 客户端控制消息。
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Ping,
    Resize {
        cols: u16,
        rows: u16,
    },
    /// 断线恢复：携带浏览器最后处理的输出序列号。
    Resume {
        last_sequence: u64,
    },
}

/// 服务端文本控制消息。
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    Connected {
        terminal_id: String,
        cols: u16,
        rows: u16,
        status: &'static str,
    },
    Pong,
    Exit {
        exit_code: i32,
    },
    Error {
        code: &'static str,
        message: String,
    },
}

impl ServerMessage {
    fn to_json(&self) -> String {
        serde_json::to_string(self).expect("server message is serializable")
    }
}

/// 读任务转发给写任务的命令。
enum WriterCommand {
    Resume(u64),
    Text(String),
}

/// 连接建立后的主循环：启动读任务并驱动写任务。
pub(crate) async fn run_socket(
    socket: WebSocket,
    session: Arc<TerminalSession>,
    handle: super::session::AttachHandle,
    auth: Arc<Auth>,
    token: String,
    ip: IpAddr,
) {
    let (mut sink, stream) = socket.split();
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<WriterCommand>(32);
    let mut watch_rx = session.subscribe_output();
    let mut cancel = handle.cancel;
    // 被新连接替换而关闭时，用自定义 close code 通知前端，避免旧窗口反复重连互踢
    let mut replaced = false;
    // None：等待客户端 resume；Some：从该序列号之后继续发送
    let mut last_sent: Option<u64> = None;

    // 读任务：浏览器 → 服务端
    let read_task = tokio::spawn(read_loop(stream, cmd_tx, session.clone(), auth, token));

    // 连接成功消息
    let (cols, rows, status) = {
        let info = session.info();
        (info.cols, info.rows, info.status)
    };
    let connected = ServerMessage::Connected {
        terminal_id: session.id.to_string(),
        cols,
        rows,
        status,
    };
    if sink.send(Message::Text(connected.to_json())).await.is_err() {
        session.detach(handle.generation);
        read_task.abort();
        return;
    }

    loop {
        tokio::select! {
            command = cmd_rx.recv() => match command {
                Some(WriterCommand::Resume(sequence)) => {
                    last_sent = Some(sequence);
                    if !drain(
                        &mut sink,
                        &mut watch_rx,
                        &session,
                        last_sent.as_mut().expect("resume sets last_sent"),
                    )
                    .await
                    {
                        break;
                    }
                }
                Some(WriterCommand::Text(text)) => {
                    if sink.send(Message::Text(text)).await.is_err() {
                        break;
                    }
                }
                // 读任务结束会 drop 发送端，recv() 返回 None
                None => break,
            },
            // 只在拿到恢复位置后才开始实时转发；缓冲是唯一数据源，之前产生的输出不会丢失
            _ = watch_rx.changed(), if last_sent.is_some() => {
                if !drain(
                    &mut sink,
                    &mut watch_rx,
                    &session,
                    last_sent.as_mut().expect("guarded by last_sent.is_some()"),
                )
                .await
                {
                    break;
                }
            },
            _ = &mut cancel => {
                replaced = true;
                break;
            }
        }
    }

    let close_frame = replaced.then(|| CloseFrame {
        code: CLOSE_REPLACED,
        reason: "replaced by another connection".into(),
    });
    let _ = sink.send(Message::Close(close_frame)).await;
    session.detach(handle.generation);
    read_task.abort();
    tracing::debug!(terminal_id = %session.id, ip = %ip, replaced, "WebSocket 已断开");
}

/// 将缓冲中 `last_sent` 之后的输出发送给客户端。
///
/// 返回 `false` 表示连接应当关闭（发送失败或 Shell 已退出）。
/// 慢客户端场景下缓冲可能已淘汰所需数据：发送 OUTPUT_GAP 并跳到最新位置，
/// 由浏览器清屏后重新从 0 恢复。
async fn drain(
    sink: &mut SplitSink<WebSocket, Message>,
    watch_rx: &mut tokio::sync::watch::Receiver<()>,
    session: &TerminalSession,
    last_sent: &mut u64,
) -> bool {
    // 先同步 watch 版本，drain 期间新产生的输出会在下一轮 changed() 中补上
    let _ = watch_rx.borrow_and_update();
    match session.replay_output(*last_sent) {
        Replay::Chunks(chunks) => {
            for chunk in chunks {
                let mut frame = Vec::with_capacity(1 + SEQUENCE_LEN + chunk.data.len());
                frame.push(OUTPUT_TYPE);
                frame.extend_from_slice(&chunk.sequence.to_be_bytes());
                frame.extend_from_slice(&chunk.data);
                if sink.send(Message::Binary(frame)).await.is_err() {
                    return false;
                }
                *last_sent = chunk.sequence;
            }
            // Shell 已退出则发送 exit 消息；Close 由 run_socket 收尾统一发送
            if let Some(code) = session.exit_code_if_exited() {
                let _ = sink
                    .send(Message::Text(ServerMessage::Exit { exit_code: code }.to_json()))
                    .await;
                return false;
            }
            true
        }
        Replay::Gap => {
            let message = ServerMessage::Error {
                code: "OUTPUT_GAP",
                message: "部分终端输出已不可用".into(),
            };
            if sink.send(Message::Text(message.to_json())).await.is_err() {
                return false;
            }
            *last_sent = session.latest_sequence();
            true
        }
    }
}

/// 读任务：接收浏览器输入与控制消息；周期性复验会话与心跳（统一判定）。
async fn read_loop(
    mut stream: SplitStream<WebSocket>,
    cmd_tx: mpsc::Sender<WriterCommand>,
    session: Arc<TerminalSession>,
    auth: Arc<Auth>,
    token: String,
) {
    let mut overflow_warned = false;
    let mut reauth = tokio::time::interval(SESSION_REVALIDATE_INTERVAL);
    reauth.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    reauth.tick().await; // 消耗立即触发的首个 tick
    let mut last_peer_message = Instant::now();

    loop {
        let message = tokio::select! {
            message = stream.next() => match message {
                Some(Ok(message)) => {
                    last_peer_message = Instant::now();
                    message
                }
                Some(Err(_)) | None => break,
            },
            _ = reauth.tick() => {
                // 会话被登出/轮换/过期后，断开已建立的连接
                if !auth.validate_session(&token) {
                    tracing::debug!(terminal_id = %session.id, "会话已失效，断开 WebSocket");
                    break;
                }
                // 心跳判死：超过 TERMINAL_PONG_TIMEOUT 未收到任何客户端消息
                //（在复验周期粒度上判定，判死点落在 pong_timeout..pong_timeout+30s）
                if last_peer_message.elapsed() >= TERMINAL_PONG_TIMEOUT {
                    tracing::debug!(terminal_id = %session.id, "心跳超时，断开 WebSocket");
                    break;
                }
                continue;
            }
        };
        match message {
            Message::Binary(data) => match data.first() {
                Some(&INPUT_TYPE) => {
                    if session.send_input(data[1..].to_vec()).is_err() {
                        // 输入通道满或已关闭：告知客户端，避免静默丢键
                        if !overflow_warned {
                            overflow_warned = true;
                            tracing::warn!(terminal_id = %session.id, "PTY 输入通道不可用或已满，输入被丢弃");
                        }
                        let _ = cmd_tx
                            .send(WriterCommand::Text(ServerMessage::Error {
                                code: "INPUT_OVERFLOW",
                                message: "输入过载，部分按键可能丢失".into(),
                            }.to_json()))
                            .await;
                    }
                }
                _ => {} // 未知二进制类型忽略
            },
            Message::Text(text) => match serde_json::from_str::<ClientMessage>(&text) {
                Ok(ClientMessage::Ping) => {
                    let _ = cmd_tx
                        .send(WriterCommand::Text(ServerMessage::Pong.to_json()))
                        .await;
                }
                Ok(ClientMessage::Resize { cols, rows }) => {
                    if let Err(message) = session.resize(cols, rows) {
                        let _ = cmd_tx
                            .send(WriterCommand::Text(ServerMessage::Error {
                                code: "RESIZE_INVALID",
                                message,
                            }.to_json()))
                            .await;
                    } else {
                        tracing::debug!(terminal_id = %session.id, cols, rows, "终端尺寸已调整");
                    }
                }
                Ok(ClientMessage::Resume { last_sequence }) => {
                    let _ = cmd_tx.send(WriterCommand::Resume(last_sequence)).await;
                }
                Err(_) => {} // 未知控制消息忽略
            },
            Message::Close(_) => break,
            // 协议层 ping/pong 由 axum 自动处理
            Message::Ping(_) | Message::Pong(_) => {}
        }
    }
}
