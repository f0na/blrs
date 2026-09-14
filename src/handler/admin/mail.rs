use axum::Json;
use axum::extract::State;
use tracing::instrument;

use crate::config::AppState;
use crate::model::err::{AppError, AppResult};
use crate::model::handler::admin::mail::{MailConfigAdmin, MailConfigInput};
use crate::model::handler::{ApiResult, JsonBody, Res, body};
use crate::repo;
use crate::util::required_text;

fn to_admin(row: &repo::mail::MailConfigAdminRow) -> MailConfigAdmin {
    MailConfigAdmin {
        id: row.id.clone(),
        host: row.host.clone(),
        port: row.port,
        username: row.username.clone(),
        from_addr: row.from_addr.clone(),
        enable: row.enable,
        has_password: row.has_password,
        create_at: crate::util::fmt_ts(row.created_at),
    }
}

fn normalize(input: MailConfigInput) -> AppResult<MailConfigInput> {
    if !(1..=65535).contains(&input.port) {
        return Err(AppError::bad_request("端口必须在 1-65535 之间"));
    }
    Ok(MailConfigInput {
        host: required_text(&input.host, "SMTP 主机", 255)?,
        port: input.port,
        username: required_text(&input.username, "SMTP 用户名", 255)?,
        password: input.password,
        from_addr: required_text(&input.from_addr, "发件人地址", 255)?,
        enable: input.enable,
    })
}

impl MailConfigInput {
    fn as_fields<'a>(&'a self, password: Option<&'a str>) -> repo::mail::MailFields<'a> {
        repo::mail::MailFields {
            host: &self.host,
            port: self.port,
            username: &self.username,
            password,
            from_addr: &self.from_addr,
            enable: self.enable,
        }
    }
}

/// SMTP 密码需要可逆存储 (要原样交给 SMTP 服务器), 所以读接口只返回 `has_password`,
/// 绝不返回密码本身。
#[instrument(skip_all)]
pub async fn get(State(st): State<AppState>) -> ApiResult<MailConfigAdmin> {
    let row = repo::mail::get_admin(&st.conns.pg)
        .await?
        .ok_or_else(|| AppError::not_found("尚未配置邮件"))?;
    Ok(Json(Res::ok(to_admin(&row))))
}

#[instrument(skip_all)]
pub async fn create(
    State(st): State<AppState>,
    payload: JsonBody<MailConfigInput>,
) -> ApiResult<MailConfigAdmin> {
    let Json(input) = body(payload)?;
    let input = normalize(input)?;

    // 新建时密码必填 —— 允许空密码等于配了个连不上的 SMTP。
    let Some(password) = input.password.as_deref().filter(|p| !p.is_empty()) else {
        return Err(AppError::bad_request("新建邮件配置时必须填写 SMTP 密码"));
    };

    let id = uuid::Uuid::now_v7().to_string();
    repo::mail::create(&st.conns.pg, &id, &input.as_fields(Some(password))).await?;

    tracing::info!(id = %id, host = %input.host, enable = input.enable, "邮件配置已创建");

    let row = repo::mail::get_admin(&st.conns.pg)
        .await?
        .ok_or_else(|| AppError::internal("邮件配置创建后读取失败"))?;
    Ok(Json(Res::ok(to_admin(&row))))
}

/// 密码留空表示保持原密码不变。
#[instrument(skip_all)]
pub async fn update(
    State(st): State<AppState>,
    payload: JsonBody<MailConfigInput>,
) -> ApiResult<MailConfigAdmin> {
    let Json(input) = body(payload)?;
    let input = normalize(input)?;

    let Some(existing) = repo::mail::get_admin(&st.conns.pg).await? else {
        return Err(AppError::not_found("尚未配置邮件, 请先调 POST"));
    };

    let password = input.password.as_deref().filter(|p| !p.is_empty());

    repo::mail::update(&st.conns.pg, &existing.id, &input.as_fields(password)).await?;

    tracing::info!(
        id = %existing.id,
        host = %input.host,
        enable = input.enable,
        password_changed = password.is_some(),
        "邮件配置已更新"
    );

    let row = repo::mail::get_admin(&st.conns.pg)
        .await?
        .ok_or_else(|| AppError::internal("邮件配置更新后读取失败"))?;
    Ok(Json(Res::ok(to_admin(&row))))
}
