//! WebSocket 终端连接：Upgrade 前认证与同源校验，协议引擎在 [`crate::terminal::ws`]。

use std::net::SocketAddr;

use axum::extract::ws::WebSocketUpgrade;
use axum::extract::{ConnectInfo, State};
use axum::response::{IntoResponse, Response};

use super::{ApiError, AuthSession, SameOrigin};
use crate::app::AppState;
use crate::config::TERMINAL_MAX_FRAME_SIZE;
use crate::terminal::ws;

/// GET /ws/terminal —— WebSocket 终端连接。
pub async fn ws_terminal(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    SameOrigin: SameOrigin,
    auth: AuthSession,
) -> Response {
    let AuthSession(token) = auth;
    let Some(session) = state.terminals.current().await else {
        return ApiError::conflict("terminal not initialized").into_response();
    };
    if !session.lifecycle().connectable() {
        return ApiError::conflict("terminal has exited").into_response();
    }
    // 升级前最后一刻仍可能被置为不可连接（回收竞态），拒绝之
    let Some(handle) = session.attach() else {
        return ApiError::conflict("terminal has closed").into_response();
    };

    tracing::debug!(terminal_id = %session.id, ip = %addr.ip(), "WebSocket 已连接");
    // 升级失败时 on_upgrade 回调不会执行，必须在这里回收 active_conn，
    // 否则 idle_since() 永远返回 None，该终端无法被空闲回收
    let generation = handle.generation;
    let failed_session = session.clone();
    ws.max_message_size(TERMINAL_MAX_FRAME_SIZE)
        .max_frame_size(TERMINAL_MAX_FRAME_SIZE)
        .on_failed_upgrade(move |_| {
            failed_session.detach(generation);
        })
        .on_upgrade(move |socket| {
            ws::run_socket(socket, session, handle, state.auth.clone(), token, addr.ip())
        })
}
