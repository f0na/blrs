use serde::{Deserialize, Serialize};

pub mod err;
pub mod handler;

/// 文章状态。
///
/// 落库就是这里字面量的原样字符串 —— **不做任何大小写或命名转换**。
/// 入参由 serde 直接反序列化校验, 非法值会被拒绝而不是静默兜底。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArticleStatus {
    Draft,
    Pub,
}

impl ArticleStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            ArticleStatus::Draft => "Draft",
            ArticleStatus::Pub => "Pub",
        }
    }
}

/// 友链状态。只有"待审核"和"已通过"两种 —— 拒绝是把记录删掉, 没有 Refuse 状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkStatus {
    Review,
    Pass,
}

impl LinkStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            LinkStatus::Review => "Review",
            LinkStatus::Pass => "Pass",
        }
    }
}
