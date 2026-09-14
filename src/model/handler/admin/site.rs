use serde::{Deserialize, Serialize};

/// 站点是单行资源: 首次 POST 创建, 之后只 PUT 更新。
#[derive(Debug, Clone, Deserialize)]
pub struct SiteInput {
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub icp: Option<String>,
    #[serde(default)]
    pub copyright: Option<String>,
    #[serde(default)]
    pub about: Option<String>,
}

/// 管理端可见的站点信息。**刻意不含 `site_secret`** —— 那一列现在是管理员密码散列。
#[derive(Debug, Clone, Serialize)]
pub struct SiteAdmin {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub icp: Option<String>,
    pub copyright: Option<String>,
    pub about: Option<String>,
    /// 时间戳 yyyy-MM-dd HH:mm:ss
    pub create_at: String,
}
