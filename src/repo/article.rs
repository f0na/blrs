use sqlx::{FromRow, PgPool, Postgres, Transaction};

use crate::model::handler::admin::article::ArticleInput;
use crate::util::now_secs;

#[derive(Debug, Clone, FromRow)]
pub struct ArticleListRow {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub cover: Option<String>,
    pub synopsis: Option<String>,
    pub likes: i64,
    pub status: String,
    pub created_at: i64,
    pub updated_at: Option<i64>,
    pub views: i64,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ArticleDetailRow {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub cover: Option<String>,
    pub synopsis: Option<String>,
    pub content: String,
    pub likes: i64,
    pub status: String,
    pub created_at: i64,
    pub updated_at: Option<i64>,
    pub deleted_at: Option<i64>,
    pub views: i64,
    pub tags: Vec<String>,
}

/// `article_view` 的一行。这是原始浏览记录, 只允许管理端读取。
#[derive(Debug, Clone, FromRow)]
pub struct ArticleViewRow {
    pub id: String,
    pub viewed_at: i64,
}

/// 列表查询的公共投影: 标签和浏览量都在 SQL 里聚合好, 绝不对每一行再查一次 (N+1)。
const LIST_PROJECTION: &str = "SELECT a.id, a.title, a.slug, a.cover, a.synopsis, a.likes, a.status, \
     a.created_at, a.updated_at, \
     COALESCE(v.cnt, 0) AS views, \
     COALESCE(t.tags, ARRAY[]::TEXT[]) AS tags \
     FROM article a \
     LEFT JOIN (SELECT article_id, COUNT(*) AS cnt FROM article_view GROUP BY article_id) v \
            ON v.article_id = a.id \
     LEFT JOIN (SELECT article_id, array_agg(tag_content ORDER BY tag_content) AS tags \
                FROM article_tag GROUP BY article_id) t \
            ON t.article_id = a.id";

const DETAIL_PROJECTION: &str = "SELECT a.id, a.title, a.slug, a.cover, a.synopsis, a.content, a.likes, \
     a.status, a.created_at, a.updated_at, a.deleted_at, \
     (SELECT COUNT(*) FROM article_view av WHERE av.article_id = a.id) AS views, \
     COALESCE((SELECT array_agg(tag_content ORDER BY tag_content) FROM article_tag \
               WHERE article_id = a.id), ARRAY[]::TEXT[]) AS tags \
     FROM article a";

// ---------------------------------------------------------------------------
// 公开读
// ---------------------------------------------------------------------------

pub async fn list_public(pg: &PgPool, limit: i64, offset: i64) -> sqlx::Result<Vec<ArticleListRow>> {
    let sql = format!(
        "{LIST_PROJECTION} WHERE a.deleted_at IS NULL AND a.status = 'Pub' \
         ORDER BY a.created_at DESC LIMIT $1 OFFSET $2"
    );
    sqlx::query_as::<_, ArticleListRow>(sqlx::AssertSqlSafe(sql))
        .persistent(false)
        .bind(limit)
        .bind(offset)
        .fetch_all(pg)
        .await
}

/// 单独一条 COUNT 而不是 `COUNT(*) OVER()`: 后者在翻到越界页时会让整个结果集为空,
/// 从而静默返回 `total = 0`。
pub async fn count_public(pg: &PgPool) -> sqlx::Result<i64> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM article WHERE deleted_at IS NULL AND status = 'Pub'",
    )
    .persistent(false)
    .fetch_one(pg)
    .await
}

pub async fn detail_public(pg: &PgPool, id: &str) -> sqlx::Result<Option<ArticleDetailRow>> {
    let sql = format!(
        "{DETAIL_PROJECTION} WHERE a.id = $1 AND a.deleted_at IS NULL AND a.status = 'Pub'"
    );
    sqlx::query_as::<_, ArticleDetailRow>(sqlx::AssertSqlSafe(sql))
        .persistent(false)
        .bind(id)
        .fetch_optional(pg)
        .await
}

/// 按 slug 取公开详情, 给前端的文章页 URL (`/<slug>`) 用。
///
/// 谓词里的 `deleted_at IS NULL` 蕴含 `uniq_article_slug_live` 的部分索引条件,
/// 所以这个查询走索引, 不是全表扫。`status = 'Pub'` 是索引外的过滤条件 ——
/// 草稿也占着 slug, 撞上了只说明这篇还没发布。
pub async fn detail_public_by_slug(
    pg: &PgPool,
    slug: &str,
) -> sqlx::Result<Option<ArticleDetailRow>> {
    let sql = format!(
        "{DETAIL_PROJECTION} WHERE a.slug = $1 AND a.deleted_at IS NULL AND a.status = 'Pub'"
    );
    sqlx::query_as::<_, ArticleDetailRow>(sqlx::AssertSqlSafe(sql))
        .persistent(false)
        .bind(slug)
        .fetch_optional(pg)
        .await
}

// ---------------------------------------------------------------------------
// 管理端读
// ---------------------------------------------------------------------------

/// 回收站和活跃列表的过滤条件。
///
/// 这里刻意写成两个**字面量** SQL 片段而不是
/// `CASE WHEN $1::bool THEN deleted_at IS NOT NULL ELSE deleted_at IS NULL END`:
/// 参数化的 CASE 表达式让 Postgres 无法证明查询蕴含索引的谓词, 于是
/// `idx_article_live_created_at` / `idx_article_trash_created_at` 这两个部分索引
/// 一个都用不上, 每次都退化成全表扫描 + 排序。
fn admin_predicate(deleted: bool) -> &'static str {
    if deleted {
        "WHERE a.deleted_at IS NOT NULL"
    } else {
        "WHERE a.deleted_at IS NULL"
    }
}

/// `deleted` 为 true 时查回收站。
///
/// 注意 `status` 仍然是可选参数 (`$1::text IS NULL OR a.status = $1`), 它不可 sargable,
/// 但索引能按 `created_at DESC` 有序扫描, 逐行过滤 status 后在 LIMIT 处提前停 ——
/// 对管理端这种带分页的列表足够了。
pub async fn list_admin(
    pg: &PgPool,
    deleted: bool,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<ArticleListRow>> {
    let sql = format!(
        "{LIST_PROJECTION} {} AND ($1::text IS NULL OR a.status = $1) \
         ORDER BY a.created_at DESC LIMIT $2 OFFSET $3",
        admin_predicate(deleted)
    );
    sqlx::query_as::<_, ArticleListRow>(sqlx::AssertSqlSafe(sql))
        .persistent(false)
        .bind(status)
        .bind(limit)
        .bind(offset)
        .fetch_all(pg)
        .await
}

pub async fn count_admin(pg: &PgPool, deleted: bool, status: Option<&str>) -> sqlx::Result<i64> {
    let sql = format!(
        "SELECT COUNT(*) FROM article a {} AND ($1::text IS NULL OR a.status = $1)",
        admin_predicate(deleted)
    );
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .persistent(false)
        .bind(status)
        .fetch_one(pg)
        .await
}

pub async fn detail_admin(pg: &PgPool, id: &str) -> sqlx::Result<Option<ArticleDetailRow>> {
    let sql = format!("{DETAIL_PROJECTION} WHERE a.id = $1");
    sqlx::query_as::<_, ArticleDetailRow>(sqlx::AssertSqlSafe(sql))
        .persistent(false)
        .bind(id)
        .fetch_optional(pg)
        .await
}

// ---------------------------------------------------------------------------
// 搜索索引
// ---------------------------------------------------------------------------

/// 单篇文章的索引投影, **只在它确实该被搜到时**才返回行。
///
/// 两条刻意的取舍:
/// - 用 `LIST_PROJECTION` 而不是 `DETAIL_PROJECTION`, 因为它不含 `content` ——
///   正文不进索引, 没必要为了建一条 1KB 的记录把 500KB 的正文读出来。
/// - "不可搜索"由 SQL 判定而不是在 Rust 里看字段: `LIST_PROJECTION` 不含 `deleted_at`,
///   在 Rust 里根本判断不了它在不在回收站。查不到行 = 该从索引里删掉, 语义只有一个。
pub async fn get_indexable(pg: &PgPool, id: &str) -> sqlx::Result<Option<ArticleListRow>> {
    let sql = format!(
        "{LIST_PROJECTION} WHERE a.id = $1 AND a.deleted_at IS NULL AND a.status = 'Pub'"
    );
    sqlx::query_as::<_, ArticleListRow>(sqlx::AssertSqlSafe(sql))
        .persistent(false)
        .bind(id)
        .fetch_optional(pg)
        .await
}

/// 全量重建索引时要推送的全部文章: 已发布且不在回收站里。
///
/// 索引里只放"能被搜到"的文章, 所以不需要任何过滤器 —— 草稿和回收站的文章根本不会
/// 进索引, 也就没有"搜出来一篇草稿"的可能。
pub async fn list_indexable(pg: &PgPool) -> sqlx::Result<Vec<ArticleListRow>> {
    let sql = format!(
        "{LIST_PROJECTION} WHERE a.deleted_at IS NULL AND a.status = 'Pub' ORDER BY a.created_at DESC"
    );
    sqlx::query_as::<_, ArticleListRow>(sqlx::AssertSqlSafe(sql))
        .persistent(false)
        .fetch_all(pg)
        .await
}

// ---------------------------------------------------------------------------
// 写
// ---------------------------------------------------------------------------

pub async fn create(tx: &mut Transaction<'_, Postgres>, id: &str, input: &ArticleInput) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO article (id, title, slug, synopsis, cover, content, likes, status, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, 0, $7, $8)",
    )
    .persistent(false)
    .bind(id)
    .bind(&input.title)
    .bind(&input.slug)
    .bind(&input.synopsis)
    .bind(&input.cover)
    .bind(&input.content)
    .bind(input.status.as_str())
    .bind(now_secs())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// 返回 false 表示目标不存在或已在回收站里。
///
/// 回收站里的文章不能改 —— 否则会出现"内容被改了但它还在回收站"这种状态,
/// 和 `soft_delete` / `restore` / `like` 的过滤条件保持一致。
pub async fn update(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    input: &ArticleInput,
) -> sqlx::Result<bool> {
    let affected = sqlx::query(
        "UPDATE article SET title = $2, slug = $3, synopsis = $4, cover = $5, content = $6, \
         status = $7, updated_at = $8 WHERE id = $1 AND deleted_at IS NULL",
    )
    .persistent(false)
    .bind(id)
    .bind(&input.title)
    .bind(&input.slug)
    .bind(&input.synopsis)
    .bind(&input.cover)
    .bind(&input.content)
    .bind(input.status.as_str())
    .bind(now_secs())
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// 整体替换标签。一条 SQL 写入, 不在 Rust 里循环 —— 循环是 N+1 的另一种写法。
pub async fn replace_tags(
    tx: &mut Transaction<'_, Postgres>,
    article_id: &str,
    tags: &[String],
) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM article_tag WHERE article_id = $1")
        .persistent(false)
        .bind(article_id)
        .execute(&mut **tx)
        .await?;

    if tags.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO article_tag (id, article_id, tag_content) \
         SELECT gen_random_uuid()::text, $1, t.tag FROM unnest($2::text[]) AS t(tag)",
    )
    .persistent(false)
    .bind(article_id)
    .bind(tags)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn soft_delete(pg: &PgPool, id: &str) -> sqlx::Result<bool> {
    let now = now_secs();
    let affected = sqlx::query(
        "UPDATE article SET deleted_at = $2, updated_at = $2 WHERE id = $1 AND deleted_at IS NULL",
    )
    .persistent(false)
    .bind(id)
    .bind(now)
    .execute(pg)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

pub async fn restore(pg: &PgPool, id: &str) -> sqlx::Result<bool> {
    let affected = sqlx::query(
        "UPDATE article SET deleted_at = NULL, updated_at = $2 WHERE id = $1 AND deleted_at IS NOT NULL",
    )
    .persistent(false)
    .bind(id)
    .bind(now_secs())
    .execute(pg)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// 真删除。migrations/0001_init.sql 里没有外键也没有 ON DELETE CASCADE, 所以标签和浏览记录必须由
/// 应用层在同一个事务里一起删掉, 否则会留下永远没人清理的孤儿行。
pub async fn hard_delete(tx: &mut Transaction<'_, Postgres>, id: &str) -> sqlx::Result<bool> {
    sqlx::query("DELETE FROM article_tag WHERE article_id = $1")
        .persistent(false)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM article_view WHERE article_id = $1")
        .persistent(false)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    let affected = sqlx::query("DELETE FROM article WHERE id = $1")
        .persistent(false)
        .bind(id)
        .execute(&mut **tx)
        .await?
        .rows_affected();
    Ok(affected > 0)
}

/// 点赞。只有已发布且未删除的文章能被点赞, 返回新的总数。
pub async fn like(pg: &PgPool, id: &str) -> sqlx::Result<Option<i64>> {
    sqlx::query_scalar(
        "UPDATE article SET likes = likes + 1 \
         WHERE id = $1 AND deleted_at IS NULL AND status = 'Pub' RETURNING likes",
    )
    .persistent(false)
    .bind(id)
    .fetch_optional(pg)
    .await
}

// ---------------------------------------------------------------------------
// 浏览记录
// ---------------------------------------------------------------------------

pub async fn insert_view(pg: &PgPool, article_id: &str, viewed_at: i64) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO article_view (id, article_id, viewed_at) VALUES ($1, $2, $3)")
        .persistent(false)
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(article_id)
        .bind(viewed_at)
        .execute(pg)
        .await?;
    Ok(())
}

pub async fn list_views(
    pg: &PgPool,
    article_id: &str,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<ArticleViewRow>> {
    sqlx::query_as::<_, ArticleViewRow>(
        "SELECT id, viewed_at FROM article_view WHERE article_id = $1 \
         ORDER BY viewed_at DESC LIMIT $2 OFFSET $3",
    )
    .persistent(false)
    .bind(article_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pg)
    .await
}

pub async fn count_views(pg: &PgPool, article_id: &str) -> sqlx::Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM article_view WHERE article_id = $1")
        .persistent(false)
        .bind(article_id)
        .fetch_one(pg)
        .await
}

/// 定时任务用: 删除 `before` 之前的浏览记录, 返回删除条数。
pub async fn cleanup_views(pg: &PgPool, before: i64) -> sqlx::Result<u64> {
    let affected = sqlx::query("DELETE FROM article_view WHERE viewed_at < $1")
        .persistent(false)
        .bind(before)
        .execute(pg)
        .await?
        .rows_affected();
    Ok(affected)
}
