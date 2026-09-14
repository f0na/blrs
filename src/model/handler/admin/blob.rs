use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct BlobTokenInput {
    /// 仅用于拼出可读的 pathname 尾部。真实路径由服务端生成, 不采信客户端给的路径。
    #[serde(default)]
    pub filename: Option<String>,
}

/// 转发给前端的 Blob 直传凭据。
///
/// 字段名用 camelCase, 和 `@vercel/blob/client` 的 `presignUrl({ clientSigningToken,
/// delegationToken }, ...)` 完全对齐, 前端可以直接把这两个值原样传进去。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobTokenRes {
    pub delegation_token: String,
    pub client_signing_token: String,
    /// 毫秒时间戳。
    pub valid_until: i64,
    /// 服务端生成的最终路径, 前端要拿它去 presign。
    pub pathname: String,
    pub access: String,
}

/// `POST /signed-token` 的原始响应。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedTokenRes {
    pub delegation_token: String,
    pub client_signing_token: String,
    pub valid_until: i64,
}
