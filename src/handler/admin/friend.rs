use axum::Json;
use axum::extract::{Path, State};
use tracing::instrument;

use crate::config::AppState;
use crate::model::LinkStatus;
use crate::model::err::{AppError, AppResult};
use crate::model::handler::admin::friend::{DecideRes, FriendLinkAdmin, FriendLinkInput, RefuseInput};
use crate::model::handler::{ApiResult, JsonBody, ListQuery, Pager, Res, body};
use crate::repo;
use crate::service;
use crate::util::{fmt_ts, optional_text, required_text, validate_email, validate_site_url};

/// 注意: 目标结构体里**没有** email 字段, 所以这里想泄露也泄露不了。
fn to_admin(row: &repo::friend::FriendLinkRow) -> FriendLinkAdmin {
    FriendLinkAdmin {
        id: row.id.clone(),
        url: row.site_url.clone(),
        name: row.site_name.clone(),
        intro: row.site_intro.clone(),
        icon: row.site_icon.clone(),
        feedback_status: parse_status(&row.feedback_status),
        create_at: fmt_ts(row.created_at),
    }
}

fn parse_status(raw: &str) -> LinkStatus {
    match raw {
        "Review" => LinkStatus::Review,
        "Pass" => LinkStatus::Pass,
        other => {
            tracing::error!(value = other, "friend_link.feedback_status 取值非法, 按 Review 处理");
            LinkStatus::Review
        }
    }
}

fn normalize(input: FriendLinkInput) -> AppResult<NormalizedFriend> {
    Ok(NormalizedFriend {
        url: validate_site_url(&input.url)?,
        name: required_text(&input.name, "站点名称", 255)?,
        intro: required_text(&input.intro, "站点介绍", 500)?,
        icon: optional_text(input.icon.as_deref(), "站点图标", 500)?,
        email: validate_email(input.email.as_deref())?,
    })
}

struct NormalizedFriend {
    url: String,
    name: String,
    intro: String,
    icon: Option<String>,
    email: Option<String>,
}

#[instrument(skip_all)]
pub async fn list(State(st): State<AppState>, q: ListQuery) -> ApiResult<Pager<FriendLinkAdmin>> {
    let (page, page_size, offset) = (q.page()?, q.page_size()?, q.offset()?);

    let status = match q.raw("status") {
        None => None,
        Some(raw) => Some(
            serde_json::from_value::<LinkStatus>(serde_json::Value::String(raw.to_string()))
                .map_err(|_| AppError::bad_request("status 只能是 Review 或 Pass"))?,
        ),
    };
    let status_filter = status.map(|s| s.as_str());

    let (rows, total) = tokio::try_join!(
        async {
            repo::friend::list_admin(&st.conns.pg, status_filter, page_size, offset)
                .await
                .map_err(AppError::from)
        },
        async {
            repo::friend::count_admin(&st.conns.pg, status_filter)
                .await
                .map_err(AppError::from)
        },
    )?;

    Ok(Json(Res::ok(Pager::new(
        page,
        page_size,
        total,
        rows.iter().map(to_admin).collect(),
    ))))
}

/// 管理端直接新增友链。管理员是主动添加的, 所以直接落成 Pass。
#[instrument(skip_all)]
pub async fn create(State(st): State<AppState>, payload: JsonBody<FriendLinkInput>) -> ApiResult<DecideRes> {
    let Json(input) = body(payload)?;
    let input = normalize(input)?;

    let new_link = repo::friend::NewFriendLink {
        url: &input.url,
        name: &input.name,
        intro: &input.intro,
        icon: input.icon.as_deref(),
        email: input.email.as_deref(),
    };
    let id = uuid::Uuid::now_v7().to_string();

    repo::friend::create(&st.conns.pg, &id, LinkStatus::Pass.as_str(), &new_link).await?;
    tracing::info!(id = %id, name = %input.name, "管理端新增友链 (直接通过)");

    Ok(Json(Res::ok(DecideRes { id, notified: false })))
}

#[instrument(skip_all, fields(friend_id = %id))]
pub async fn update(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: JsonBody<FriendLinkInput>,
) -> ApiResult<DecideRes> {
    let Json(input) = body(payload)?;
    let input = normalize(input)?;

    let new_link = repo::friend::NewFriendLink {
        url: &input.url,
        name: &input.name,
        intro: &input.intro,
        icon: input.icon.as_deref(),
        email: None,
    };

    if !repo::friend::update(&st.conns.pg, &id, &new_link).await? {
        return Err(AppError::not_found("友链不存在"));
    }
    tracing::info!(id = %id, "友链已更新");

    Ok(Json(Res::ok(DecideRes { id, notified: false })))
}

/// 审核通过, 并发通知邮件。
///
/// 业务操作 (状态变更) 在发信之前就已经提交, 所以邮件发不出去**不会**回滚审核结果,
/// 响应里用 `notified` 说明邮件到底发出去没有。
#[instrument(skip_all, fields(friend_id = %id))]
pub async fn approve(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<DecideRes> {
    let decision = service::friend::approve(&st, &id).await?;
    Ok(Json(Res::ok(DecideRes {
        id,
        notified: decision.notified,
    })))
}

/// 拒绝。带上管理员编辑的说明, 发信之后删掉记录。
#[instrument(skip_all, fields(friend_id = %id))]
pub async fn refuse(
    State(st): State<AppState>,
    Path(id): Path<String>,
    payload: JsonBody<RefuseInput>,
) -> ApiResult<DecideRes> {
    let Json(input) = body(payload)?;
    let decision = service::friend::refuse(&st, &id, &input.message).await?;
    Ok(Json(Res::ok(DecideRes {
        id,
        notified: decision.notified,
    })))
}

/// 静默删除, 不发通知。给垃圾提交用 —— 拒绝那条路径是一定会发信的。
#[instrument(skip_all, fields(friend_id = %id))]
pub async fn delete(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<DecideRes> {
    if !repo::friend::delete(&st.conns.pg, &id).await? {
        return Err(AppError::not_found("友链不存在"));
    }
    tracing::warn!(id = %id, "友链已静默删除 (未发送通知)");
    Ok(Json(Res::ok(DecideRes {
        id,
        notified: false,
    })))
}
