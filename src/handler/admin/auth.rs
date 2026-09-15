use axum::extract::State;
use axum::http::HeaderMap;
use axum::{Extension, Json};
use tracing::instrument;

use crate::config::{AppState, LOGIN_MAX_FAILURES};
use crate::model::err::AppError;
use crate::model::handler::admin::auth::{LoginInput, LoginRes, MeRes, PasswordInput};
use crate::model::handler::{ApiResult, JsonBody, Res, body};
use crate::service;
use crate::service::auth::AdminClaims;
use crate::util::trusted_client_ip;

/// 管理员登录。
///
/// 两种模式:
/// - **库里还没设密码**: 不做任何限制直接放行 (按需求), 打 WARN 提示去设置密码。
/// - **已设密码**: 密码错误累计到 [`LOGIN_MAX_FAILURES`] 次就把这个 IP 封 90 天,
///   封禁期间该 IP 访问**任何**接口都会被最外层的中间件拦下。
#[instrument(skip_all)]
pub async fn login(
    State(st): State<AppState>,
    headers: HeaderMap,
    payload: JsonBody<LoginInput>,
) -> ApiResult<LoginRes> {
    let Json(input) = body(payload)?;
    // 只有可信来源才参与计数/封禁, 否则伪造一个头就能把别人封掉
    let ip = trusted_client_ip(&headers);

    let password_set = service::auth::password_is_set(&st).await?;

    if password_set {
        if !service::auth::verify_login(&st, input.pswd.as_deref()).await? {
            match ip.as_deref() {
                Some(ip) => match service::ban::record_failure(&st, ip).await {
                    Some(count) => {
                        tracing::warn!(ip = %ip, failures = count, "管理员登录失败");
                        if count >= LOGIN_MAX_FAILURES {
                            service::ban::ban(&st, ip).await;
                        }
                    }
                    None => tracing::warn!(ip = %ip, "管理员登录失败 (失败计数不可用, 未计数)"),
                },
                None => tracing::warn!("管理员登录失败 (无法确定可信 IP, 未计数)"),
            }
            return Err(AppError::unauthorized("密码错误"));
        }
        if let Some(ip) = ip.as_deref() {
            service::ban::clear_failures(&st, ip).await;
        }
    } else {
        tracing::warn!("管理员密码尚未设置, 本次登录免密通过");
    }

    let (token, expire_at) = service::auth::issue_token(&st.cfg)?;
    tracing::info!(expire_at, password_set, "管理员登录成功");

    Ok(Json(Res::ok(LoginRes {
        token,
        expire_at,
        password_set,
    })))
}

/// 设置或修改管理员密码。需要有效 JWT。
#[instrument(skip_all)]
pub async fn set_password(
    State(st): State<AppState>,
    payload: JsonBody<PasswordInput>,
) -> ApiResult<()> {
    let Json(input) = body(payload)?;
    service::auth::set_password(&st, &input.pswd).await?;
    Ok(Json(Res::ok(())))
}

#[instrument(skip_all)]
pub async fn me(Extension(claims): Extension<AdminClaims>) -> ApiResult<MeRes> {
    Ok(Json(Res::ok(MeRes {
        expire_at: claims.exp,
    })))
}
