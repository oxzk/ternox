//! 应用状态、路由注册与静态资源服务（rust-embed 嵌入前端构建产物）。

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Path as AxumPath, Request};
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rust_embed::RustEmbed;
use serde_json::json;

use crate::api;
use crate::auth::Auth;
use crate::config::Config;
use crate::terminal::manager::TerminalManager;

/// 共享应用状态（通过 Axum State 注入）。
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub auth: Arc<Auth>,
    pub terminals: TerminalManager,
}

/// 嵌入的前端静态资源：目录为 backend/static（Docker 构建时由前端 dist 覆盖）。
#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/static"]
struct Assets;

/// HTTP 请求体大小上限（WebSocket 消息大小由 WS 配置约束）。
const BODY_LIMIT: usize = 64 * 1024;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/api/login", post(api::auth::login))
        .route("/api/logout", post(api::auth::logout))
        .route("/api/session", get(api::auth::session))
        .route("/api/terminal", post(api::terminal::init_terminal))
        .route("/ws/terminal", get(api::websocket::ws_terminal))
        .route("/healthz", get(healthz))
        .route("/assets/*path", get(asset))
        .fallback(fallback)
        .layer(middleware::from_fn(security_headers))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(state)
}

/// 为所有响应补充基础安全头。
async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        // 前端为嵌入式已知产物：无内联脚本，样式允许内联（xterm 动态样式）
        HeaderValue::from_static(
            "default-src 'self'; connect-src 'self' ws: wss:; img-src 'self' data:; \
             style-src 'self' 'unsafe-inline'; script-src 'self'; object-src 'none'; \
             base-uri 'none'; frame-ancestors 'none'",
        ),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

/// GET /healthz —— Docker 健康检查，无需认证。
async fn healthz() -> &'static str {
    "ok"
}

async fn asset(AxumPath(path): AxumPath<String>) -> Response {
    // 通配捕获的是 /assets/ 之后的剩余路径，rust-embed 的键带 assets/ 前缀
    serve_asset(&format!("assets/{path}"))
}

fn serve_asset(path: &str) -> Response {
    match Assets::get(path) {
        Some(content) => {
            // vite 构建的 assets 带内容哈希，可长期缓存
            let cache_control = if path.starts_with("assets/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            (
                [
                    (header::CONTENT_TYPE, mime_of(path)),
                    (header::CACHE_CONTROL, cache_control),
                ],
                content.data.to_vec(),
            )
                .into_response()
        }
        None => not_found(),
    }
}

async fn fallback(uri: Uri) -> Response {
    // API / WS 未知路径返回 404
    if uri.path().starts_with("/api/") || uri.path().starts_with("/ws/") {
        return not_found();
    }
    // public 目录产生的根级静态文件（favicon.svg、robots.txt 等）
    let path = uri.path().trim_start_matches('/');
    if !path.is_empty() && Assets::get(path).is_some() {
        return serve_asset(path);
    }
    // 其余回落到前端入口
    serve_asset("index.html")
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" }))).into_response()
}

fn mime_of(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
