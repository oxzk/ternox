//! 终端初始化 API。

use axum::extract::State;
use axum::Json;

use super::{ApiError, AuthSession, SameOrigin};
use crate::app::AppState;
use crate::terminal::session::TerminalInfo;

/// POST /api/terminal —— 获取或初始化唯一终端。
///
/// 未创建则创建 PTY 与 Shell；已运行则返回现有状态；已退出则允许重新初始化。
/// 不允许客户端指定 Shell 路径。
pub async fn init_terminal(
    State(state): State<AppState>,
    _same_origin: SameOrigin,
    _auth: AuthSession,
) -> Result<Json<TerminalInfo>, ApiError> {
    let session = state
        .terminals
        .get_or_create()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(session.info()))
}
