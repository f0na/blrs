use serde::{Deserialize, Serialize};

/// 公开的文章列表项。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeArticleList {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub cover: Option<String>,
    pub synopsis: Option<String>,
    pub likes: i64,
    /// 浏览量**数量**。原始的逐条查看记录 (article_view) 只有管理端能读。
    pub views: i64,
    /// Unix 秒, 原样返回。
    pub create_at: i64,
    /// Unix 秒, 原样返回。
    pub update_at: Option<i64>,
    pub tags: Vec<String>,
}

/// 公开的文章详情。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArticleDetail {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub cover: Option<String>,
    pub synopsis: Option<String>,
    pub content: String,
    pub likes: i64,
    /// 浏览量**数量**, 不是逐条记录。
    pub views: i64,
    /// Unix 秒, 原样返回。
    pub create_at: i64,
    /// Unix 秒, 原样返回。
    pub update_at: Option<i64>,
    pub tags: Vec<String>,
}
