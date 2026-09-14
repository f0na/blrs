use serde::{Deserialize, Serialize};

/// 登录入参。
///
/// 库里还没设置过密码时, `pswd` 传空 (或不传) 即可通过 —— 之后必须用拿到的 token
/// 调 `POST /admin/password` 设置密码。
#[derive(Debug, Clone, Deserialize)]
pub struct LoginInput {
    #[serde(default)]
    pub pswd: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoginRes {
    pub token: String,
    /// 时间戳 yyyy-MM-dd HH:mm:ss
    pub expire_at: String,
    /// 为 false 说明尚未设置密码, 前端应当引导去设置。
    pub password_set: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PasswordInput {
    pub pswd: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MeRes {
    pub expire_at: String,
}
