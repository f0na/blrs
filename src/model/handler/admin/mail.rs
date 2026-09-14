use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct MailConfigInput {
    pub host: String,
    pub port: i32,
    pub username: String,
    /// 更新时留空表示保持原密码不变。
    #[serde(default)]
    pub password: Option<String>,
    pub from_addr: String,
    pub enable: bool,
}

/// 管理端可见的邮件配置。
///
/// SMTP 密码需要可逆存储 (必须原样交给 SMTP 服务器), 但**任何读接口都不返回它**,
/// 只用一个布尔说明有没有配过。
#[derive(Debug, Clone, Serialize)]
pub struct MailConfigAdmin {
    pub id: String,
    pub host: String,
    pub port: i32,
    pub username: String,
    pub from_addr: String,
    pub enable: bool,
    pub has_password: bool,
    /// 时间戳 yyyy-MM-dd HH:mm:ss
    pub create_at: String,
}
