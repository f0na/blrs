use axum::Json;
use axum::extract::State;
use tracing::instrument;

use crate::config::AppState;
use crate::model::err::{AppError, AppResult};
use crate::model::handler::admin::site::{SiteAdmin, SiteInput};
use crate::model::handler::{ApiResult, JsonBody, Res, body};
use crate::repo;
use crate::repo::site::SiteRow;
use crate::service;
use crate::util::{fmt_ts, optional_text, required_text};

fn to_admin(row: &SiteRow) -> SiteAdmin {
    SiteAdmin {
        id: row.id.clone(),
        name: row.site_name.clone().unwrap_or_default(),
        icon: row.site_icon.clone(),
        banner: row.banner_image.clone(),
        icp: row.icp.clone(),
        copyright: row.copyright.clone(),
        about: row.about_content.clone(),
        create_at: fmt_ts(row.created_at),
    }
}

/// 校验并收敛入参。字段长度对齐 migrations/0001_init.sql 里的列宽。
struct NormalizedSite {
    name: String,
    icon: Option<String>,
    banner: Option<String>,
    icp: Option<String>,
    copyright: Option<String>,
    about: Option<String>,
}

fn normalize(input: SiteInput) -> AppResult<NormalizedSite> {
    Ok(NormalizedSite {
        name: required_text(&input.name, "站点名称", 255)?,
        icon: optional_text(input.icon.as_deref(), "站点图标", 500)?,
        banner: optional_text(input.banner.as_deref(), "Banner 图", 500)?,
        icp: optional_text(input.icp.as_deref(), "ICP 备案号", 100)?,
        copyright: optional_text(input.copyright.as_deref(), "版权信息", 255)?,
        about: optional_text(input.about.as_deref(), "关于内容", 20_000)?,
    })
}

impl NormalizedSite {
    fn as_fields(&self) -> repo::site::SiteFields<'_> {
        repo::site::SiteFields {
            name: &self.name,
            icon: self.icon.as_deref(),
            banner: self.banner.as_deref(),
            icp: self.icp.as_deref(),
            copyright: self.copyright.as_deref(),
            about: self.about.as_deref(),
        }
    }
}

#[instrument(skip_all)]
pub async fn get(State(st): State<AppState>) -> ApiResult<SiteAdmin> {
    let row = repo::site::get(&st.conns.pg)
        .await?
        .ok_or_else(|| AppError::not_found("站点尚未创建"))?;
    Ok(Json(Res::ok(to_admin(&row))))
}

#[instrument(skip_all)]
pub async fn create(State(st): State<AppState>, payload: JsonBody<SiteInput>) -> ApiResult<SiteAdmin> {
    let Json(input) = body(payload)?;
    let input = normalize(input)?;

    let id = uuid::Uuid::now_v7().to_string();
    let created = repo::site::create(&st.conns.pg, &id, &input.as_fields()).await?;

    if !created {
        return Err(AppError::conflict("站点已存在, 请改用 PUT 更新"));
    }

    service::site::invalidate(&st).await;
    tracing::info!(id = %id, name = %input.name, "站点已创建");

    let row = repo::site::get(&st.conns.pg)
        .await?
        .ok_or_else(|| AppError::internal("站点创建后读取失败"))?;
    Ok(Json(Res::ok(to_admin(&row))))
}

#[instrument(skip_all)]
pub async fn update(State(st): State<AppState>, payload: JsonBody<SiteInput>) -> ApiResult<SiteAdmin> {
    let Json(input) = body(payload)?;
    let input = normalize(input)?;

    let Some(existing) = repo::site::get(&st.conns.pg).await? else {
        return Err(AppError::not_found("站点尚未创建, 请先调 POST"));
    };

    repo::site::update(&st.conns.pg, &existing.id, &input.as_fields()).await?;

    service::site::invalidate(&st).await;
    tracing::info!(id = %existing.id, "站点信息已更新");

    let row = repo::site::get(&st.conns.pg)
        .await?
        .ok_or_else(|| AppError::internal("站点更新后读取失败"))?;
    Ok(Json(Res::ok(to_admin(&row))))
}
