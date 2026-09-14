use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use tracing::instrument;

use crate::config::AppState;
use crate::model::ArticleStatus;
use crate::model::err::{AppError, AppResult};
use crate::model::handler::admin::article::{
    ArticleAdminDetail, ArticleAdminList, ArticleInput, ArticleViewItem,
};
use crate::model::handler::{ApiResult, JsonBody, ListQuery, Pager, Res, body};
use crate::repo;
use crate::service;
use crate::util::{fmt_ts, optional_text, required_text};

#[derive(Debug, Serialize)]
pub struct IdRes {
    pub id: String,
}

fn to_admin_list(row: &repo::article::ArticleListRow) -> ArticleAdminList {
    ArticleAdminList {
        id: row.id.clone(),
        title: row.title.clone(),
        slug: row.slug.clone(),
        cover: row.cover.clone(),
        synopsis: row.synopsis.clone(),
        likes: row.likes,
        views: row.views,
        status: parse_status(&row.status),
        create_at: fmt_ts(row.created_at),
        update_at: row.updated_at.map(fmt_ts),
        // 列表投影不带 deleted_at, 单独补一次会让每行多一次查询, 所以列表里恒为 None。
        // 回收站视图本身已经由 deleted 参数表达。
        deleted_at: None,
        tags: row.tags.clone(),
    }
}

fn to_admin_detail(row: &repo::article::ArticleDetailRow) -> ArticleAdminDetail {
    ArticleAdminDetail {
        id: row.id.clone(),
        title: row.title.clone(),
        slug: row.slug.clone(),
        cover: row.cover.clone(),
        synopsis: row.synopsis.clone(),
        content: row.content.clone(),
        likes: row.likes,
        views: row.views,
        status: parse_status(&row.status),
        create_at: fmt_ts(row.created_at),
        update_at: row.updated_at.map(fmt_ts),
        deleted_at: row.deleted_at.map(fmt_ts),
        tags: row.tags.clone(),
    }
}

/// 库里 status 只可能是我们写进去的两个字面量之一。
/// 出现别的值说明有人手改过库 —— 记 ERROR 并按 Draft 处理, 不让管理端列表直接 500。
fn parse_status(raw: &str) -> ArticleStatus {
    match raw {
        "Draft" => ArticleStatus::Draft,
        "Pub" => ArticleStatus::Pub,
        other => {
            tracing::error!(value = other, "article.status 取值非法, 按 Draft 处理");
            ArticleStatus::Draft
        }
    }
}

fn normalize(input: ArticleInput) -> AppResult<ArticleInput> {
    let title = required_text(&input.title, "标题", 255)?;
    let slug = required_text(&input.slug, "URL 别名", 255)?;
    if slug.contains('/') || slug.contains('?') || slug.contains('#') {
        return Err(AppError::bad_request("URL 别名不能包含 / ? #"));
    }

    Ok(ArticleInput {
        title,
        slug,
        cover: optional_text(input.cover.as_deref(), "封面图", 500)?,
        synopsis: required_text(&input.synopsis, "摘要", 5_000)?,
        content: required_text(&input.content, "正文", 500_000)?,
        tags: service::article::sanitize_tags(input.tags),
        status: input.status,
    })
}

#[instrument(skip_all)]
pub async fn list(State(st): State<AppState>, q: ListQuery) -> ApiResult<Pager<ArticleAdminList>> {
    let (page, page_size, offset) = (q.page()?, q.page_size()?, q.offset()?);
    let deleted = q.bool("deleted")?.unwrap_or(false);

    // status 也走 serde 校验, 和写入路径同一套规则: 非法值直接拒, 不做大小写转换。
    let status = match q.raw("status") {
        None => None,
        Some(raw) => Some(
            serde_json::from_value::<ArticleStatus>(serde_json::Value::String(raw.to_string()))
                .map_err(|_| AppError::bad_request("status 只能是 Draft 或 Pub"))?,
        ),
    };
    let status_filter = status.map(|s| s.as_str());

    let (rows, total) = tokio::try_join!(
        async {
            repo::article::list_admin(&st.conns.pg, deleted, status_filter, page_size, offset)
                .await
                .map_err(AppError::from)
        },
        async {
            repo::article::count_admin(&st.conns.pg, deleted, status_filter)
                .await
                .map_err(AppError::from)
        },
    )?;

    tracing::debug!(page, page_size, total, deleted, ?status_filter, "文章列表查询完成");

    Ok(Json(Res::ok(Pager::new(
        page,
        page_size,
        total,
        rows.iter().map(to_admin_list).collect(),
    ))))
}

#[instrument(skip_all, fields(article_id = %id))]
pub async fn detail(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<ArticleAdminDetail> {
    let row = repo::article::detail_admin(&st.conns.pg, &id)
        .await?
        .ok_or_else(|| AppError::not_found("文章不存在"))?;
    Ok(Json(Res::ok(to_admin_detail(&row))))
}

#[instrument(skip_all, fields(title = %input_title(&payload)))]
pub async fn create(State(st): State<AppState>, payload: JsonBody<ArticleInput>) -> ApiResult<IdRes> {
    let Json(input) = body(payload)?;
    let input = normalize(input)?;

    let id = uuid::Uuid::now_v7().to_string();
    let mut tx = st.conns.pg.begin().await?;
    repo::article::create(&mut tx, &id, &input).await?;
    repo::article::replace_tags(&mut tx, &id, &input.tags).await?;
    tx.commit().await?;

    // 事务提交之后才同步索引 —— 先索引再提交的话, 提交失败就会在索引里留下一篇不存在的文章。
    service::search::sync(&st, &id).await;

    tracing::info!(id = %id, status = input.status.as_str(), tags = input.tags.len(), "文章已创建");
    Ok(Json(Res::ok(IdRes { id })))
}

#[instrument(skip_all, fields(article_id = %id))]
pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: JsonBody<ArticleInput>,
) -> ApiResult<IdRes> {
    let Json(input) = body(payload)?;
    let input = normalize(input)?;

    let mut tx = st.conns.pg.begin().await?;
    let updated = repo::article::update(&mut tx, &id, &input).await?;
    if !updated {
        // 回滚掉本次事务里的任何改动再报 404
        tx.rollback().await?;
        return Err(AppError::not_found("文章不存在"));
    }
    repo::article::replace_tags(&mut tx, &id, &input.tags).await?;
    tx.commit().await?;

    // 改成草稿时 sync 会把它从索引里删掉, 不用在这里分情况。
    service::search::sync(&st, &id).await;

    tracing::info!(id = %id, status = input.status.as_str(), "文章已更新");
    Ok(Json(Res::ok(IdRes { id })))
}

/// 软删除 (进回收站)。
#[instrument(skip_all, fields(article_id = %id))]
pub async fn soft_delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<IdRes> {
    if !repo::article::soft_delete(&st.conns.pg, &id).await? {
        return Err(AppError::not_found("文章不存在或已在回收站"));
    }
    // 回收站里的文章不该被搜到。
    service::search::sync(&st, &id).await;
    tracing::info!(id = %id, "文章已移入回收站");
    Ok(Json(Res::ok(IdRes { id })))
}

#[instrument(skip_all, fields(article_id = %id))]
pub async fn restore(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<IdRes> {
    if !repo::article::restore(&st.conns.pg, &id).await? {
        return Err(AppError::not_found("文章不存在或不在回收站"));
    }
    // 恢复出来是草稿的话 sync 不会写索引, 这里不用判断原状态。
    service::search::sync(&st, &id).await;
    tracing::info!(id = %id, "文章已从回收站恢复");
    Ok(Json(Res::ok(IdRes { id })))
}

/// 真删除。标签和浏览记录在同一个事务里一起清掉。
#[instrument(skip_all, fields(article_id = %id))]
pub async fn hard_delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<IdRes> {
    let mut tx = st.conns.pg.begin().await?;
    if !repo::article::hard_delete(&mut tx, &id).await? {
        tx.rollback().await?;
        return Err(AppError::not_found("文章不存在"));
    }
    tx.commit().await?;

    // 库里已经查不到这一行, sync 会把它从索引里删掉。
    service::search::sync(&st, &id).await;

    tracing::warn!(id = %id, "文章已彻底删除 (含标签与浏览记录)");
    Ok(Json(Res::ok(IdRes { id })))
}

/// 逐条浏览记录。这是 `article_view` 原始数据的**唯一**出口。
#[instrument(skip_all, fields(article_id = %id))]
pub async fn list_views(
    State(st): State<AppState>,
    Path(id): Path<String>,
    q: ListQuery,
) -> ApiResult<Pager<ArticleViewItem>> {
    let (page, page_size, offset) = (q.page()?, q.page_size()?, q.offset()?);

    let (rows, total) = tokio::try_join!(
        async {
            repo::article::list_views(&st.conns.pg, &id, page_size, offset)
                .await
                .map_err(AppError::from)
        },
        async {
            repo::article::count_views(&st.conns.pg, &id)
                .await
                .map_err(AppError::from)
        },
    )?;

    let list = rows
        .iter()
        .map(|row| ArticleViewItem {
            id: row.id.clone(),
            view_at: fmt_ts(row.viewed_at),
        })
        .collect();

    Ok(Json(Res::ok(Pager::new(page, page_size, total, list))))
}

/// 只是为了让 `#[instrument]` 能在解析请求体之前拿到一个标题字段 ——
/// 解析失败时也能在日志里看到请求的是哪个标题。
fn input_title(payload: &JsonBody<ArticleInput>) -> String {
    payload
        .as_ref()
        .ok()
        .map(|json| json.title.clone())
        .unwrap_or_default()
}
