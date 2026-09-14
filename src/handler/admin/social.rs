use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use tracing::instrument;

use crate::config::AppState;
use crate::model::err::{AppError, AppResult};
use crate::model::handler::admin::social::{SocialAdmin, SocialInput};
use crate::model::handler::site::SocialType;
use crate::model::handler::{ApiResult, JsonBody, Res, body};
use crate::repo;
use crate::repo::social::SocialRow;
use crate::service;
use crate::util::{fmt_ts, optional_text, required_text};

#[derive(Debug, Serialize)]
pub struct IdRes {
    pub id: String,
}

fn to_admin(row: &SocialRow) -> SocialAdmin {
    SocialAdmin {
        id: row.id.clone(),
        social_type: parse_type(&row.social_type, &row.id),
        icon: row.icon.clone(),
        value: row.value.clone(),
        create_at: fmt_ts(row.created_at),
    }
}

fn parse_type(raw: &str, id: &str) -> SocialType {
    match raw {
        "Account" => SocialType::Account,
        "Link" => SocialType::Link,
        other => {
            tracing::error!(id = %id, value = other, "social.type 取值非法, 按 Link 处理");
            SocialType::Link
        }
    }
}

fn type_str(value: SocialType) -> &'static str {
    match value {
        SocialType::Account => "Account",
        SocialType::Link => "Link",
    }
}

fn normalize(input: SocialInput) -> AppResult<(SocialType, Option<String>, String)> {
    Ok((
        input.social_type,
        optional_text(input.icon.as_deref(), "图标", 100)?,
        required_text(&input.value, "值", 500)?,
    ))
}

#[instrument(skip_all)]
pub async fn list(State(st): State<AppState>) -> ApiResult<Vec<SocialAdmin>> {
    let rows = repo::social::list(&st.conns.pg).await?;
    Ok(Json(Res::ok(rows.iter().map(to_admin).collect())))
}

#[instrument(skip_all)]
pub async fn create(State(st): State<AppState>, payload: JsonBody<SocialInput>) -> ApiResult<IdRes> {
    let Json(input) = body(payload)?;
    let (social_type, icon, value) = normalize(input)?;

    let id = uuid::Uuid::now_v7().to_string();
    repo::social::create(&st.conns.pg, &id, type_str(social_type), icon.as_deref(), &value).await?;

    service::site::invalidate(&st).await;
    tracing::info!(id = %id, social_type = type_str(social_type), "社交链接已创建");

    Ok(Json(Res::ok(IdRes { id })))
}

#[instrument(skip_all, fields(social_id = %id))]
pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: JsonBody<SocialInput>,
) -> ApiResult<IdRes> {
    let Json(input) = body(payload)?;
    let (social_type, icon, value) = normalize(input)?;

    if !repo::social::update(&st.conns.pg, &id, type_str(social_type), icon.as_deref(), &value).await? {
        return Err(AppError::not_found("社交链接不存在"));
    }

    service::site::invalidate(&st).await;
    tracing::info!(id = %id, "社交链接已更新");

    Ok(Json(Res::ok(IdRes { id })))
}

#[instrument(skip_all, fields(social_id = %id))]
pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<IdRes> {
    if !repo::social::delete(&st.conns.pg, &id).await? {
        return Err(AppError::not_found("社交链接不存在"));
    }

    service::site::invalidate(&st).await;
    tracing::info!(id = %id, "社交链接已删除");

    Ok(Json(Res::ok(IdRes { id })))
}
