use crate::config::{
    AppState, BLOB_ALLOWED_CONTENT_TYPES, BLOB_MAX_SIZE_BYTES, BLOB_TOKEN_TTL_MS,
};
use crate::model::err::{AppError, AppResult};
use crate::model::handler::admin::blob::{BlobTokenRes, SignedTokenRes};
use crate::util::{now_millis, sanitize_filename};

const BLOB_API_BASE: &str = "https://blob.vercel-storage.com";
const BLOB_API_VERSION: &str = "4";

/// 图片一律当公开资源 (博客正文要直接引用)。
const BLOB_ACCESS: &str = "public";

/// 签发限时的 Blob 直传凭据。
///
/// 走的是 Vercel Blob 的 HTTP 接口 `/signed-token`, 不是在 Rust 里做签名 ——
/// `vercel_blob` 这个库没有签发能力, 但 API 有, 所以这里直接构造请求。
/// 大小、类型、有效期、操作全部由服务端写死, 前端拿到的令牌只能按这个范围上传。
pub async fn issue_token(st: &AppState, filename: Option<&str>) -> AppResult<BlobTokenRes> {
    let pathname = build_pathname(filename);
    let valid_until = now_millis() + BLOB_TOKEN_TTL_MS;

    let body = serde_json::json!({
        "pathname": pathname,
        "operations": ["put"],
        "validUntil": valid_until,
        "maximumSizeInBytes": BLOB_MAX_SIZE_BYTES,
        "allowedContentTypes": BLOB_ALLOWED_CONTENT_TYPES,
    });

    tracing::info!(
        pathname = %pathname,
        valid_until,
        max_bytes = BLOB_MAX_SIZE_BYTES,
        ttl_ms = BLOB_TOKEN_TTL_MS,
        "向 blob api 申请直传令牌"
    );

    let response = st
        .http
        .post(format!("{BLOB_API_BASE}/signed-token"))
        .header("authorization", format!("Bearer {}", st.cfg.blob_token))
        .header("x-api-version", BLOB_API_VERSION)
        .header("x-vercel-blob-store-id", &st.cfg.blob_store_id)
        .json(&body)
        .send()
        .await?;

    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        // 只记状态码和响应体, 绝不记 token
        tracing::error!(status = status.as_u16(), body = %text, "blob api 签发令牌失败");
        return Err(AppError::upstream(format!(
            "blob 签发令牌失败, HTTP {}",
            status.as_u16()
        )));
    }

    // 注意: 成功响应的 body 里就是令牌本身, 所以这里只记长度和错误, 绝不打印内容。
    let signed: SignedTokenRes = serde_json::from_str(&text).map_err(|e| {
        tracing::error!(error = %e, body_len = text.len(), "blob api 响应无法解析");
        AppError::upstream("blob 签发令牌响应格式异常")
    })?;

    tracing::info!(pathname = %pathname, "直传令牌已签发");

    Ok(BlobTokenRes {
        delegation_token: signed.delegation_token,
        client_signing_token: signed.client_signing_token,
        valid_until: signed.valid_until,
        pathname,
        access: BLOB_ACCESS.to_string(),
    })
}

/// 路径完全由服务端生成: 客户端给的只是一个经过收敛的文件名片段, 拼在一个新 uuid 后面。
/// 这样客户端无法指定 (或覆盖) 任何已有对象。
fn build_pathname(filename: Option<&str>) -> String {
    let safe = sanitize_filename(filename.unwrap_or(""));
    format!("uploads/{}/{}", uuid::Uuid::now_v7(), safe)
}

#[cfg(test)]
mod tests {
    use super::build_pathname;

    #[test]
    fn pathname_is_server_generated_and_prefix_scoped() {
        let p = build_pathname(Some("../../etc/passwd"));
        assert!(p.starts_with("uploads/"));
        assert!(p.ends_with("etc_passwd"));
        // 收敛之后不可能再出现路径穿越
        assert!(!p.contains(".."));
        // 只有 `uploads/<uuid>/<name>` 这两个分隔符, 文件名片段里不可能再夹路径
        assert_eq!(p.matches('/').count(), 2);
    }

    #[test]
    fn pathname_handles_missing_filename() {
        assert!(build_pathname(None).ends_with("/file"));
    }
}
