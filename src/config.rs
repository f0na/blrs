use std::env;
use std::sync::Arc;
use std::time::Duration;

use crate::db::Connections;

/// JWT 有效期: 3 天。
pub const JWT_TTL_SECS: i64 = 3 * 24 * 3600;

/// Blob 直传令牌的有效期: 5 分钟。前端拿到后要立刻上传。
pub const BLOB_TOKEN_TTL_MS: i64 = 5 * 60 * 1000;

/// Blob 直传的大小上限: 25MB。
pub const BLOB_MAX_SIZE_BYTES: i64 = 25 * 1024 * 1024;

/// Blob 直传允许的内容类型 (常见图片格式)。
pub const BLOB_ALLOWED_CONTENT_TYPES: &[&str] = &[
    "image/jpeg",
    "image/png",
    "image/gif",
    "image/webp",
    "image/avif",
    "image/svg+xml",
    "image/bmp",
    "image/x-icon",
];

/// 同一 IP 的计数去重窗口: 1 小时。浏览和点赞共用。
pub const VIEW_DEDUPE_SECS: u64 = 3600;

/// 浏览记录保留天数。
pub const VIEW_RETENTION_DAYS: i64 = 90;

/// 登录失败多少次后封禁。
pub const LOGIN_MAX_FAILURES: i64 = 3;

/// 封禁时长: 90 天。
pub const BAN_SECS: u64 = 90 * 24 * 3600;

/// Algolia 的三把钥匙。
///
/// 分成 search / write 两把是 Algolia 的既定模型, 不是一个索引一份配置:
/// 搜索接口只拿得到 search key, 写索引的 write key 永远不出现在读路径上。
#[derive(Clone, Debug)]
pub struct AlgoliaEnv {
    pub app_id: String,
    pub search_key: String,
    pub write_key: String,
}

/// 站点信息的环境变量形态 (迁移之后、库里还没有填过内容的 site 记录时注入一条)。
///
/// 只有名称/图标/Banner/版权这四项有变量, ICP 和关于内容没有, 所以注入出来的记录
/// 这两项为空, 之后在管理端补。`site_name` 没配就整个跳过注入, 公开接口仍然按
/// "站点尚未初始化" 报错。
#[derive(Clone, Debug)]
pub struct SiteEnv {
    pub name: Option<String>,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub copyright: Option<String>,
}

/// SMTP 配置的环境变量形态 (库里没有启用的 mail_config 时回退到这里)。
#[derive(Clone, Debug)]
pub struct MailEnv {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from: String,
}

pub struct AppConfig {
    pub jwt_secret: String,
    pub cron_secret: String,
    pub site: SiteEnv,
    pub mail: MailEnv,
    pub algolia: AlgoliaEnv,
    pub blob_token: String,
    pub blob_store_id: String,
    pub cors_origins: Vec<String>,
}

impl AppConfig {
    /// 启动时校验全部必需环境变量。
    ///
    /// 邮件相关变量的缺失必须**在启动时**就炸掉, 而不是等第一次发信才发现 ——
    /// 友链审核那条路径失败一次就丢一封通知邮件。
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let blob_token = env_required("BLOB_READ_WRITE_TOKEN")?;
        let blob_store_id = match env_optional("BLOB_STORE_ID") {
            Some(v) => v,
            // `vercel_blob_rw_<storeId>_<secret>`, 取第 4 段
            None => blob_token
                .split('_')
                .nth(3)
                .filter(|s| !s.is_empty())
                .ok_or("无法从 BLOB_READ_WRITE_TOKEN 解析 storeId, 请显式设置 BLOB_STORE_ID")?
                .to_string(),
        };

        let mail_port = env_required("mail_port")?
            .parse::<u16>()
            .map_err(|e| format!("mail_port 不是合法端口: {e}"))?;

        let cors_origins: Vec<String> = env_required("CORS_ORIGINS")?
            .split(',')
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(Self {
            jwt_secret: env_required("JWT_SECRET")?,
            cron_secret: env_required("CRON_SECRET")?,
            // 站点信息以库里的 site 记录为准, 这几个变量只在库里还没填过内容时注入
            // 一条, 所以不校验必填。
            site: SiteEnv {
                name: env_optional("site_name"),
                icon: env_optional("site_icon"),
                // 变量叫 site_banner, 对应的列是 banner_image。
                banner: env_optional("site_banner"),
                copyright: env_optional("site_copyright"),
            },
            mail: MailEnv {
                host: env_required("mail_host")?,
                port: mail_port,
                username: env_required("mail_username")?,
                password: env_required("mail_password")?,
                from: env_required("mail_from")?,
            },
            algolia: AlgoliaEnv {
                app_id: env_required("algolia_app_id")?,
                search_key: env_required("algolia_search_key")?,
                write_key: env_required("algolia_write_key")?,
            },
            blob_token,
            blob_store_id,
            cors_origins,
        })
    }
}

fn env_required(key: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    match env::var(key) {
        Ok(v) if !v.trim().is_empty() => Ok(v.trim().to_string()),
        _ => Err(format!("缺少必需的环境变量: {key}").into()),
    }
}

fn env_optional(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// 全应用共享状态。
#[derive(Clone)]
pub struct AppState {
    pub conns: Connections,
    pub cfg: Arc<AppConfig>,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new(conns: Connections, cfg: AppConfig) -> Self {
        // 调用 Blob 签发接口用的客户端。连接复用, 超时收紧。
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|e| {
                tracing::error!(error = %e, "构建 http 客户端失败, 退回到默认配置");
                reqwest::Client::new()
            });
        Self {
            conns,
            cfg: Arc::new(cfg),
            http,
        }
    }
}
