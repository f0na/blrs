use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::Response;

use crate::config::AppState;
use crate::model::err::AppError;
use crate::util::constant_time_eq;

/// Vercel 定时任务的守卫。
///
/// 平台触发 cron 时会带上 `Authorization: Bearer <CRON_SECRET>`, 所以这里不需要
/// 再给定时任务单独发明一套凭据。
pub async fn require_cron(
    State(st): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let provided = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or_default();

    if !constant_time_eq(provided, &st.cfg.cron_secret) {
        tracing::warn!(path = %req.uri().path(), "cron 鉴权失败");
        return Err(AppError::unauthorized("invalid cron credentials"));
    }

    Ok(next.run(req).await)
}
