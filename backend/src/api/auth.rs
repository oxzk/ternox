//! 登录、登出与会话状态 API。

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::{ApiError, AuthSession, SameOrigin};
use crate::app::AppState;
use crate::auth::{
    clear_session_cookie, rate_limit_key, session_cookie, token_from_headers, LoginOutcome,
};

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// POST /api/login —— 环境变量账号密码登录，成功后轮换 Session Token。
pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    SameOrigin: SameOrigin,
    Json(payload): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    let previous = token_from_headers(&headers);
    let limit_key = rate_limit_key(addr.ip(), &headers);
    match state.auth.login(
        previous.as_deref(),
        &payload.username,
        &payload.password,
        limit_key,
    ) {
        LoginOutcome::Ok(token) => {
            tracing::info!(ip = %addr.ip(), "登录成功");
            let cookie = session_cookie(
                &token,
                crate::config::SESSION_TTL.as_secs(),
                state.config.session_secure,
            );
            Ok((
                [(header::SET_COOKIE, cookie)],
                Json(json!({ "authenticated": true })),
            )
                .into_response())
        }
        LoginOutcome::InvalidCredentials => {
            tracing::warn!(ip = %addr.ip(), "登录失败：凭据错误");
            Err(ApiError::unauthorized("invalid username or password"))
        }
        LoginOutcome::RateLimited(retry_after) => {
            let retry = retry_after.to_string();
            tracing::warn!(ip = %addr.ip(), retry_after, "登录触发限流");
            Ok((
                StatusCode::TOO_MANY_REQUESTS,
                [("Retry-After", retry.as_str())],
                Json(json!({ "error": "尝试次数过多，请稍后再试" })),
            )
                .into_response())
        }
    }
}

/// POST /api/logout —— 销毁 Session 并清除 Cookie；不终止 PTY 与 Shell。
pub async fn logout(
    State(state): State<AppState>,
    SameOrigin: SameOrigin,
    AuthSession(token): AuthSession,
) -> Result<Response, ApiError> {
    if !state.auth.destroy_session(&token) {
        return Err(ApiError::unauthorized("session expired"));
    }
    tracing::info!("会话已销毁（终端进程不受影响）");
    let cookie = clear_session_cookie(state.config.session_secure);
    Ok((
        [(header::SET_COOKIE, cookie)],
        Json(json!({ "authenticated": false })),
    )
        .into_response())
}

/// GET /api/session —— 检查当前 Cookie 是否有效（始终返回 200）。
pub async fn session(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let authenticated = token_from_headers(&headers)
        .map(|token| state.auth.validate_session(&token))
        .unwrap_or(false);
    Json(json!({ "authenticated": authenticated })).into_response()
}
