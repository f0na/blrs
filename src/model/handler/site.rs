use serde::{Deserialize, Serialize};

/// 首页 / 文章页用的站点信息 (不含 about 正文)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeSite {
    pub name: String,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub icp: Option<String>,
    pub copyright: Option<String>,
}

/// 关于页用的站点信息, 比首页多 about 正文和建站时间。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AboutSite {
    pub name: String,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub icp: Option<String>,
    pub copyright: Option<String>,
    pub about: Option<String>,
    /// Unix 秒, 原样返回。
    pub create_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SocialType {
    Account,
    Link,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeSocial {
    pub social_type: SocialType,
    /// 库里允许为空, 所以这里也是 Option —— 不要为了前端好看而在读取时 panic。
    pub icon: Option<String>,
    pub value: String,
}
