use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};

use crate::model::err::AppResult;
use crate::util::now_secs;

/// site 表的一行。
///
/// **刻意不含 `site_secret`** —— 那一列现在是管理员密码散列, 绝不能进 SELECT 列表。
/// 需要读写密码散列时走本模块下面专门的 `password_hash` / `set_password_hash`。
///
/// 带 serde 是因为站点上下文要整个塞进 Valkey 缓存。
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SiteRow {
    pub id: String,
    /// DDL 里允许为空 (首次设密码时会先建一条只有 id 的记录), 所以这里必须是 Option。
    pub site_name: Option<String>,
    pub site_icon: Option<String>,
    pub banner_image: Option<String>,
    pub icp: Option<String>,
    pub copyright: Option<String>,
    pub about_content: Option<String>,
    pub created_at: i64,
}

/// 站点信息在公开接口和 `/admin/site` 都要读, 只有这一份列清单。
const SELECT_SITE: &str = "SELECT id, site_name, site_icon, banner_image, icp, copyright, \
                           about_content, created_at \
                           FROM site ORDER BY created_at ASC LIMIT 1";

/// 读取站点 (单行资源, 取最早创建的那条)。
pub async fn get(pg: &PgPool) -> AppResult<Option<SiteRow>> {
    Ok(sqlx::query_as::<_, SiteRow>(SELECT_SITE)
        .persistent(false)
        .fetch_optional(pg)
        .await?)
}

/// 站点的可写字段。
pub struct SiteFields<'a> {
    pub name: &'a str,
    pub icon: Option<&'a str>,
    pub banner: Option<&'a str>,
    pub icp: Option<&'a str>,
    pub copyright: Option<&'a str>,
    pub about: Option<&'a str>,
}

/// 首次创建站点。站点是单行资源, 已经填过内容的直接返回 false。
///
/// 这里要处理一个坑: 首次设置管理员密码时 [`set_password_hash`] 会先建一条只有
/// id/created_at/password 的占位记录 (密码总得有地方存)。如果这里简单地
/// "存在即冲突", 管理员就会永远卡在 409 上, 再也填不了站点信息。
/// 所以先尝试把这条例占位记录补全, 补不上再插新行。
pub async fn create(pg: &PgPool, id: &str, fields: &SiteFields<'_>) -> AppResult<bool> {
    // 1. 补全占位记录
    let filled = sqlx::query(
        "UPDATE site SET site_name = $1, site_icon = $2, banner_image = $3, icp = $4, \
         copyright = $5, about_content = $6 \
         WHERE id = (SELECT id FROM site ORDER BY created_at ASC LIMIT 1) \
           AND site_name IS NULL",
    )
    .persistent(false)
    .bind(fields.name)
    .bind(fields.icon)
    .bind(fields.banner)
    .bind(fields.icp)
    .bind(fields.copyright)
    .bind(fields.about)
    .execute(pg)
    .await?
    .rows_affected();
    if filled > 0 {
        tracing::info!("已补全首次设置密码时创建的占位站点记录");
        return Ok(true);
    }

    // 2. 一行都没有, 正常插入
    let inserted = sqlx::query(
        "INSERT INTO site (id, site_name, site_icon, banner_image, icp, copyright, about_content, created_at) \
         SELECT $1, $2, $3, $4, $5, $6, $7, $8 \
         WHERE NOT EXISTS (SELECT 1 FROM site)",
    )
    .persistent(false)
    .bind(id)
    .bind(fields.name)
    .bind(fields.icon)
    .bind(fields.banner)
    .bind(fields.icp)
    .bind(fields.copyright)
    .bind(fields.about)
    .bind(now_secs())
    .execute(pg)
    .await?
    .rows_affected();

    Ok(inserted > 0)
}

pub async fn update(pg: &PgPool, id: &str, fields: &SiteFields<'_>) -> AppResult<bool> {
    let affected = sqlx::query(
        "UPDATE site SET site_name = $2, site_icon = $3, banner_image = $4, icp = $5, \
         copyright = $6, about_content = $7 WHERE id = $1",
    )
    .persistent(false)
    .bind(id)
    .bind(fields.name)
    .bind(fields.icon)
    .bind(fields.banner)
    .bind(fields.icp)
    .bind(fields.copyright)
    .bind(fields.about)
    .execute(pg)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// 管理员密码散列 (即 `site.site_secret`)。返回 None 表示尚未设置密码。
pub async fn password_hash(pg: &PgPool) -> AppResult<Option<String>> {
    let value: Option<Option<String>> = sqlx::query_scalar(
        "SELECT site_secret FROM site ORDER BY created_at ASC LIMIT 1",
    )
    .persistent(false)
    .fetch_optional(pg)
    .await?;
    Ok(value.flatten().filter(|s| !s.is_empty()))
}

/// 写入密码散列。站点行还不存在时顺手建一条 —— 否则首次设密码无处可存。
pub async fn set_password_hash(pg: &PgPool, hash: &str) -> AppResult<()> {
    let affected = sqlx::query(
        "UPDATE site SET site_secret = $1 \
         WHERE id = (SELECT id FROM site ORDER BY created_at ASC LIMIT 1)",
    )
    .persistent(false)
    .bind(hash)
    .execute(pg)
    .await?
    .rows_affected();

    if affected == 0 {
        sqlx::query("INSERT INTO site (id, created_at, site_secret) VALUES ($1, $2, $3)")
            .persistent(false)
            .bind(uuid::Uuid::now_v7().to_string())
            .bind(now_secs())
            .bind(hash)
            .execute(pg)
            .await?;
        tracing::warn!("站点记录不存在, 已为存储密码散列自动创建一条 site 记录");
    }
    Ok(())
}
