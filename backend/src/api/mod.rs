//! HTTP API 与 WebSocket 路由处理。

pub mod auth;
pub mod terminal;
pub mod websocket;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::app::AppState;
use crate::auth::token_from_headers;

/// 统一 API 错误响应：`{"error": "..."}`。
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, message)
    }

    pub fn forbidden() -> Self {
        Self::new(StatusCode::FORBIDDEN, "cross-origin request rejected")
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

/// 已认证会话提取器：统一完成 Cookie 解析与服务端 Session 校验，
/// 替代各 handler 中重复的认证前置代码。
pub struct AuthSession(pub String);

#[axum::async_trait]
impl FromRequestParts<AppState> for AuthSession {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = token_from_headers(&parts.headers)
            .ok_or_else(|| ApiError::unauthorized("not authenticated"))?;
        if !state.auth.validate_session(&token) {
            return Err(ApiError::unauthorized("session expired"));
        }
        Ok(Self(token))
    }
}

/// 同源校验提取器：CSRF / WebSocket Origin 防护的统一入口，
/// 所有变更类端点必须携带，避免漏写造成约定式安全缺口。
pub struct SameOrigin;

#[axum::async_trait]
impl FromRequestParts<AppState> for SameOrigin {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        if !same_origin(&parts.headers) {
            return Err(ApiError::forbidden());
        }
        Ok(Self)
    }
}

/// 同源校验（CSRF / WebSocket Origin 防护）。
///
/// 浏览器跨域请求会携带 Origin 头；非浏览器客户端不携带，予以放行。
/// Origin 为 `null` 或与 Host（或 X-Forwarded-Host）不一致时拒绝。
pub fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true;
    };
    if origin.eq_ignore_ascii_case("null") {
        return false;
    }
    let (scheme, rest) = if let Some(rest) = origin.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = origin.strip_prefix("http://") {
        ("http", rest)
    } else {
        return false;
    };
    let Some(origin_host_port) = rest.split(['/', '?', '#']).next() else {
        return false;
    };
    if origin_host_port.is_empty() {
        return false;
    }
    let default_port = if scheme == "https" { "443" } else { "80" };
    let (origin_host, origin_port) = split_host_port(origin_host_port, default_port);
    let origin_port = origin_port.unwrap_or_else(|| default_port.to_string());

    let Some(host) = host_of(headers) else {
        return false;
    };
    let (host_name, host_port) = split_host_port(&host, "");
    if !host_name.eq_ignore_ascii_case(&origin_host) {
        return false;
    }
    // Host 未显式携带端口时，仅当 Origin 使用协议默认端口才视为同源
    match host_port {
        Some(port) => port == origin_port,
        None => origin_port == default_port,
    }
}

fn host_of(headers: &HeaderMap) -> Option<String> {
    // 只信 Host 头：X-Forwarded-Host 在反代误配透传时可被客户端控制，架空 Origin 校验
    headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
}

/// 拆分 host 与 port；未显式携带端口时返回 fallback（可为空表示未提供）。
fn split_host_port(value: &str, fallback: &str) -> (String, Option<String>) {
    // IPv6 字面量 [::1]:8443
    if let Some(rest) = value.strip_prefix('[') {
        if let Some((host, tail)) = rest.split_once(']') {
            let port = tail
                .strip_prefix(':')
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .or_else(|| (!fallback.is_empty()).then(|| fallback.to_string()));
            return (host.to_string(), port);
        }
    }
    match value.rsplit_once(':') {
        Some((host, port))
            if !host.is_empty()
                && !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit()) =>
        {
            (host.to_string(), Some(port.to_string()))
        }
        _ => (
            value.to_string(),
            (!fallback.is_empty()).then(|| fallback.to_string()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    fn request(host: &str, origin: Option<&str>) -> bool {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, host.parse().unwrap());
        if let Some(origin) = origin {
            headers.insert(header::ORIGIN, origin.parse().unwrap());
        }
        same_origin(&headers)
    }

    #[test]
    fn same_origin_checks() {
        assert!(request("localhost:7681", None));
        assert!(request("localhost:7681", Some("http://localhost:7681")));
        assert!(request("localhost:7681", Some("http://localhost")));
        assert!(request("example.com", Some("https://example.com")));
        assert!(!request("localhost:7681", Some("http://evil.com")));
        assert!(!request("localhost:7681", Some("http://localhost:9999")));
        assert!(!request("localhost:7681", Some("null")));
    }
}
