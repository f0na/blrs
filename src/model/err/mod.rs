use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::model::handler::Res;

/// 业务错误码。同时决定 HTTP 状态码与响应体里的 `code` 字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrCode {
    Ok,
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    Unprocessable,
    /// 数据库错误
    Db,
    /// 上游依赖错误 (Blob API, SMTP 等)
    Upstream,
    /// 服务端内部错误 (含配置错误)
    Internal,
}

impl ErrCode {
    pub const fn as_i32(self) -> i32 {
        match self {
            ErrCode::Ok => 0,
            ErrCode::BadRequest => 40000,
            ErrCode::Unauthorized => 40100,
            ErrCode::Forbidden => 40300,
            ErrCode::NotFound => 40400,
            ErrCode::Conflict => 40900,
            ErrCode::Unprocessable => 42200,
            ErrCode::Internal => 50000,
            ErrCode::Db => 50001,
            ErrCode::Upstream => 50002,
        }
    }

    pub const fn http(self) -> StatusCode {
        match self {
            ErrCode::Ok => StatusCode::OK,
            ErrCode::BadRequest => StatusCode::BAD_REQUEST,
            ErrCode::Unauthorized => StatusCode::UNAUTHORIZED,
            ErrCode::Forbidden => StatusCode::FORBIDDEN,
            ErrCode::NotFound => StatusCode::NOT_FOUND,
            ErrCode::Conflict => StatusCode::CONFLICT,
            ErrCode::Unprocessable => StatusCode::UNPROCESSABLE_ENTITY,
            ErrCode::Internal | ErrCode::Db => StatusCode::INTERNAL_SERVER_ERROR,
            // 我们自己是好的, 是依赖 (Blob API / SMTP) 出了问题 —— 502 比 500 更准确,
            // 也能让前端和运维一眼区分开。
            ErrCode::Upstream => StatusCode::BAD_GATEWAY,
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;

/// 应用统一错误。
///
/// 契约: `msg` 是**唯一**会返回给客户端的内容, `internal` 只进日志。
/// 任何可能泄漏实现细节 (SQL 语句、连接串、上游响应体) 的信息都必须放 `internal`。
#[derive(Debug)]
pub struct AppError {
    pub code: ErrCode,
    pub msg: String,
    pub internal: String,
    pub retryable: bool,
}

impl AppError {
    fn new(code: ErrCode, msg: impl Into<String>, internal: impl Into<String>) -> Self {
        Self {
            code,
            msg: msg.into(),
            internal: internal.into(),
            retryable: false,
        }
    }

    /// 客户端入参错误。`msg` 就是校验失败的说明, 不含内部信息。
    pub fn bad_request(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        Self::new(ErrCode::BadRequest, msg.clone(), msg)
    }

    pub fn unauthorized(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        Self::new(ErrCode::Unauthorized, msg.clone(), msg)
    }

    pub fn forbidden(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        Self::new(ErrCode::Forbidden, msg.clone(), msg)
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        Self::new(ErrCode::NotFound, msg.clone(), msg)
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        Self::new(ErrCode::Conflict, msg.clone(), msg)
    }

    pub fn unprocessable(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        Self::new(ErrCode::Unprocessable, msg.clone(), msg)
    }

    /// 服务端内部错误。客户端只看到通用的 "internal server error"。
    pub fn internal(internal: impl Into<String>) -> Self {
        Self::new(ErrCode::Internal, "internal server error", internal)
    }

    pub fn db(internal: impl Into<String>) -> Self {
        Self::new(ErrCode::Db, "internal server error", internal)
    }

    pub fn upstream(internal: impl Into<String>) -> Self {
        Self::new(ErrCode::Upstream, "upstream service error", internal)
    }

    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {} ({})", self.code.as_i32(), self.msg, self.internal)
    }
}

impl std::error::Error for AppError {}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // 这条日志会落在请求的 `http` span 里, 自动带上 request_id / path / client_ip
        if self.code.http().is_server_error() {
            tracing::error!(
                code = self.code.as_i32(),
                retryable = self.retryable,
                detail = %self.internal,
                "请求处理失败"
            );
        } else {
            tracing::warn!(code = self.code.as_i32(), detail = %self.internal, "请求被拒绝");
        }

        let body = Res::<()> {
            code: self.code.as_i32(),
            msg: self.msg,
            data: None,
        };
        (self.code.http(), Json(body)).into_response()
    }
}

// ---------------------------------------------------------------------------
// 外部错误 -> AppError 的映射
// ---------------------------------------------------------------------------

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        let detail = e.to_string();
        match &e {
            sqlx::Error::RowNotFound => AppError::not_found("资源不存在"),
            sqlx::Error::Database(db) => match db.code().as_deref() {
                Some("23505") => AppError::conflict("记录已存在"),
                Some("23503") | Some("23502") => AppError::unprocessable("数据不满足完整性约束"),
                _ => AppError::db(detail),
            },
            _ => AppError::db(detail),
        }
    }
}

impl From<jsonwebtoken::errors::Error> for AppError {
    fn from(e: jsonwebtoken::errors::Error) -> Self {
        use jsonwebtoken::errors::ErrorKind;
        match e.kind() {
            // 过期是正常情况, 单独给前端一个可区分的提示, 便于触发自动跳登录
            ErrorKind::ExpiredSignature => AppError::unauthorized("token expired"),
            _ => Self::new(ErrCode::Unauthorized, "invalid token", e.to_string()),
        }
    }
}

impl From<redis::RedisError> for AppError {
    fn from(e: redis::RedisError) -> Self {
        AppError::internal(format!("valkey: {e}")).retryable()
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        AppError::upstream(format!("http client: {e}")).retryable()
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::bad_request(format!("请求体不是合法 JSON: {e}"))
    }
}

impl From<std::env::VarError> for AppError {
    fn from(e: std::env::VarError) -> Self {
        AppError::internal(format!("环境变量缺失: {e}"))
    }
}

impl From<lettre::error::Error> for AppError {
    fn from(e: lettre::error::Error) -> Self {
        AppError::upstream(format!("lettre: {e}"))
    }
}

impl From<lettre::transport::smtp::Error> for AppError {
    fn from(e: lettre::transport::smtp::Error) -> Self {
        AppError::upstream(format!("smtp: {e}"))
    }
}

/// `vercel_blob` 的 `Result` 别名是 `pub(crate)`, 所以这里必须写全名。
/// 认证类/商店类错误是**服务端配置问题**, 一律映射成 500 而不是 403 —— 绝不能
/// 让浏览器以为是自己没权限。
impl From<vercel_blob::error::VercelBlobError> for AppError {
    fn from(e: vercel_blob::error::VercelBlobError) -> Self {
        use vercel_blob::error::VercelBlobError as E;
        match e {
            E::BlobNotFound() => AppError::not_found("文件不存在"),
            E::BadRequest(msg) => AppError::bad_request(msg),
            E::InvalidInput(msg) => AppError::bad_request(msg),
            E::NotAuthenticated() | E::Forbidden() | E::StoreNotFound() | E::StoreSuspended() => {
                AppError::internal(format!("blob 配置错误: {e}"))
            }
            E::HttpError(_) | E::UnknownError(_, _) => {
                AppError::upstream(format!("blob api: {e}")).retryable()
            }
        }
    }
}
