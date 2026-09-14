use axum::Json;
use axum::extract::State;
use serde::Serialize;
use tracing::instrument;

use crate::config::AppState;
use crate::model::handler::{ApiResult, Res};
use crate::service;

#[derive(Debug, Serialize)]
pub struct ReindexRes {
    /// 推送到索引的文章条数。
    pub indexed: usize,
}

/// 全量重建搜索索引。文章的增删改会自动同步, 这个接口是用来修复漂移的。
#[instrument(skip_all)]
pub async fn reindex(State(st): State<AppState>) -> ApiResult<ReindexRes> {
    let indexed = service::search::reindex(&st).await?;
    tracing::info!(indexed, "搜索索引已全量重建");
    Ok(Json(Res::ok(ReindexRes { indexed })))
}

/// 清空搜索索引。清完搜索会返回空结果, 必须跟一次全量重建才能恢复 ——
/// 所以这里用 WARN 记一笔, 免得日志里看不出搜索为什么突然没结果。
#[instrument(skip_all)]
pub async fn clear(State(st): State<AppState>) -> ApiResult<()> {
    service::search::clear_index(&st).await?;
    tracing::warn!("搜索索引已被管理端清空, 需要全量重建才能恢复");
    Ok(Json(Res::ok(())))
}
