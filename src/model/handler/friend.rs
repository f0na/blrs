use serde::{Deserialize, Serialize};

/// 公开的友链。刻意不含 email 字段 —— 通知邮箱任何读接口都不得返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendLink {
    pub url: String,
    pub name: String,
    pub intro: String,
    pub icon: Option<String>,
}

/// 访客提交友链申请。
#[derive(Debug, Clone, Deserialize)]
pub struct FriendApplyInput {
    pub url: String,
    pub name: String,
    pub intro: String,
    pub icon: Option<String>,
    /// 通知邮箱。只写不读, 落库后不会再通过任何接口返回。
    #[serde(default)]
    pub email: Option<String>,
}
