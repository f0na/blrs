use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::Response;

use crate::config::AppState;
use crate::model::err::AppError;
use crate::service;

/// 管理端 JWT 守卫。
///
/// 通过后会把 claims 塞进 request extensions, handler 用 `Extension<AdminClaims>` 取。
/// 只挂在管理端路由上, 且 `/admin/login` 不在这组路由里 —— 见 `axum.rs` 的装配。
pub async fn require_admin(
    State(st): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| AppError::unauthorized("缺少 Authorization: Bearer <token>"))?;

    let claims = service::auth::verify_token(&st.cfg, token)
        .inspect_err(|e| tracing::warn!(reason = %e, "管理端鉴权失败"))?;

    req.extensions_mut().insert(claims);
    Ok(next.run(req).await)
}
