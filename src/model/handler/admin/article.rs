use serde::{Deserialize, Serialize};

use crate::model::ArticleStatus;

#[derive(Debug, Clone, Deserialize)]
pub struct ArticleInput {
    pub title: String,
    pub slug: String,
    #[serde(default)]
    pub cover: Option<String>,
    pub synopsis: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// 状态用枚举反序列化: 传 `"draft"` 会直接被拒绝, 不做任何大小写转换。
    pub status: ArticleStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArticleAdminList {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub cover: Option<String>,
    pub synopsis: Option<String>,
    pub likes: i64,
    pub views: i64,
    pub status: ArticleStatus,
    /// Unix 秒, 原样返回。
    pub create_at: i64,
    /// Unix 秒, 原样返回。
    pub update_at: Option<i64>,
    /// 非空表示在回收站里。
    /// Unix 秒, 原样返回。
    pub deleted_at: Option<i64>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArticleAdminDetail {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub cover: Option<String>,
    pub synopsis: Option<String>,
    pub content: String,
    pub likes: i64,
    pub views: i64,
    pub status: ArticleStatus,
    /// Unix 秒, 原样返回。
    pub create_at: i64,
    /// Unix 秒, 原样返回。
    pub update_at: Option<i64>,
    /// Unix 秒, 原样返回。
    pub deleted_at: Option<i64>,
    pub tags: Vec<String>,
}

/// 单条浏览记录。这是 `article_view` 原始数据的**唯一**出口, 只在管理端出现。
#[derive(Debug, Clone, Serialize)]
pub struct ArticleViewItem {
    pub id: String,
    /// Unix 秒, 原样返回。
    pub view_at: i64,
}
