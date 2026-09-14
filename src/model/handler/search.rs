use serde::Serialize;

/// 搜索结果项。
///
/// 字段全部来自 Algolia 里的索引快照, **不回查数据库** —— 所以刻意不含 `likes` /
/// `views`: 这两个数一直在变, 放进索引就等于每次点赞、每次浏览都得重新索引一遍。
/// 前端要点赞数/浏览量时按 `id` 去调 `/articles/{id}`。
#[derive(Debug, Clone, Serialize)]
pub struct SearchArticle {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub cover: Option<String>,
    pub synopsis: Option<String>,
    /// 时间戳 yyyy-MM-dd HH:mm:ss
    pub create_at: String,
    /// 时间戳 yyyy-MM-dd HH:mm:ss
    pub update_at: Option<String>,
    pub tags: Vec<String>,
}
