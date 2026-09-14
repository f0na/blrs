use axum::Json;
use axum::extract::State;
use tracing::instrument;

use crate::config::AppState;
use crate::model::handler::admin::blob::{BlobTokenInput, BlobTokenRes};
use crate::model::handler::{ApiResult, JsonBody, Res, body};
use crate::service;

/// 签发限时 Blob 直传凭据。
///
/// 这个路由挂在管理端守卫之下, 所以**只有登录校验通过才会签发**。
/// 后端本身不接收文件, 只给前端一把限时限量的钥匙。
#[instrument(skip_all)]
pub async fn issue_token(
    State(st): State<AppState>,
    payload: JsonBody<BlobTokenInput>,
) -> ApiResult<BlobTokenRes> {
    let Json(input) = body(payload)?;
    let token = service::blob::issue_token(&st, input.filename.as_deref()).await?;
    Ok(Json(Res::ok(token)))
}
