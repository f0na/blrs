use axum::Json;
use axum::extract::State;
use serde::Serialize;
use tracing::instrument;

use crate::config::{AppState, VIEW_RETENTION_DAYS};
use crate::model::handler::{ApiResult, Res};
use crate::repo;
use crate::util::now_secs;

#[derive(Debug, Serialize)]
pub struct CleanupRes {
    /// 删掉的记录条数。
    deleted: u64,
    /// 早于这个时间戳的记录都被删掉了。
    before: i64,
}

/// 清理过期的浏览记录。由 Vercel Cron 每天触发一次, 用 `CRON_SECRET` 鉴权。
///
/// 删除是幂等的: 多跑一次不会产生额外影响, 所以频率调整不影响正确性。
#[instrument(skip_all)]
pub async fn cleanup_views(State(st): State<AppState>) -> ApiResult<CleanupRes> {
    let before = now_secs() - VIEW_RETENTION_DAYS * 86400;
    let deleted = repo::article::cleanup_views(&st.conns.pg, before).await?;

    tracing::info!(
        deleted,
        before,
        retention_days = VIEW_RETENTION_DAYS,
        "浏览记录清理完成"
    );

    Ok(Json(Res::ok(CleanupRes { deleted, before })))
}
