use sqlx::{FromRow, PgPool, Postgres, Transaction};

use crate::util::now_secs;

/// 友链的一行。
///
/// **刻意不含 `email`** —— 通知邮箱禁止被任何读接口返回, 连字段都不该存在。
/// 只有审核/拒绝流程需要邮箱, 那条路径走下面的 [`get_for_update`]。
#[derive(Debug, Clone, FromRow)]
pub struct FriendLinkRow {
    pub id: String,
    pub site_name: String,
    pub site_url: String,
    pub site_intro: String,
    pub site_icon: Option<String>,
    pub feedback_status: String,
    pub created_at: i64,
}

/// 全代码库**唯一**带 `email` 的结构体。
///
/// 它只在 `service::friend` 的审核/拒绝事务里被读取, 邮箱值不会离开那个函数。
#[derive(Debug, Clone, FromRow)]
pub struct FriendLinkMailRow {
    pub site_name: String,
    pub site_url: String,
    pub feedback_status: String,
    pub email: Option<String>,
}

pub struct NewFriendLink<'a> {
    pub url: &'a str,
    pub name: &'a str,
    pub intro: &'a str,
    pub icon: Option<&'a str>,
    pub email: Option<&'a str>,
}

const SELECT_FRIEND: &str = "SELECT id, site_name, site_url, site_intro, site_icon, feedback_status, \
                             created_at FROM friend_link";

/// 公开列表: 只有审核通过的。
pub async fn list_public(pg: &PgPool, limit: i64, offset: i64) -> sqlx::Result<Vec<FriendLinkRow>> {
    let sql = format!(
        "{SELECT_FRIEND} WHERE feedback_status = 'Pass' ORDER BY created_at DESC LIMIT $1 OFFSET $2"
    );
    sqlx::query_as::<_, FriendLinkRow>(sqlx::AssertSqlSafe(sql))
        .persistent(false)
        .bind(limit)
        .bind(offset)
        .fetch_all(pg)
        .await
}

pub async fn count_public(pg: &PgPool) -> sqlx::Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM friend_link WHERE feedback_status = 'Pass'")
        .persistent(false)
        .fetch_one(pg)
        .await
}

/// 管理端列表: 全部状态。
pub async fn list_admin(
    pg: &PgPool,
    status: Option<&str>,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<FriendLinkRow>> {
    let sql = format!(
        "{SELECT_FRIEND} WHERE ($1::text IS NULL OR feedback_status = $1) \
         ORDER BY created_at DESC LIMIT $2 OFFSET $3"
    );
    sqlx::query_as::<_, FriendLinkRow>(sqlx::AssertSqlSafe(sql))
        .persistent(false)
        .bind(status)
        .bind(limit)
        .bind(offset)
        .fetch_all(pg)
        .await
}

pub async fn count_admin(pg: &PgPool, status: Option<&str>) -> sqlx::Result<i64> {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM friend_link WHERE ($1::text IS NULL OR feedback_status = $1)",
    )
    .persistent(false)
    .bind(status)
    .fetch_one(pg)
    .await
}

pub async fn create(pg: &PgPool, id: &str, status: &str, input: &NewFriendLink<'_>) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO friend_link (id, site_name, site_url, site_intro, site_icon, email, \
         feedback_status, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .persistent(false)
    .bind(id)
    .bind(input.name)
    .bind(input.url)
    .bind(input.intro)
    .bind(input.icon)
    .bind(input.email)
    .bind(status)
    .bind(now_secs())
    .execute(pg)
    .await?;
    Ok(())
}

/// 管理端编辑。邮箱不在可编辑字段里 —— 一旦落库就不再暴露也不再修改。
pub async fn update(pg: &PgPool, id: &str, input: &NewFriendLink<'_>) -> sqlx::Result<bool> {
    let affected = sqlx::query(
        "UPDATE friend_link SET site_name = $2, site_url = $3, site_intro = $4, site_icon = $5 \
         WHERE id = $1",
    )
    .persistent(false)
    .bind(id)
    .bind(input.name)
    .bind(input.url)
    .bind(input.intro)
    .bind(input.icon)
    .execute(pg)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

pub async fn delete(pg: &PgPool, id: &str) -> sqlx::Result<bool> {
    let affected = sqlx::query("DELETE FROM friend_link WHERE id = $1")
        .persistent(false)
        .bind(id)
        .execute(pg)
        .await?
        .rows_affected();
    Ok(affected > 0)
}

// ---------------------------------------------------------------------------
// 审核 / 拒绝: 只有这里会碰 email
// ---------------------------------------------------------------------------

/// 只读一次邮箱, 不加锁、不开事务。
/// 拒绝流程要在发信前用它取地址, 且发信必须在事务之外进行 —— 否则会占着数据库
/// 连接等 SMTP, 而连接池只有一条连接。
pub async fn get_mail_row(pg: &PgPool, id: &str) -> sqlx::Result<Option<FriendLinkMailRow>> {
    sqlx::query_as::<_, FriendLinkMailRow>(
        "SELECT site_name, site_url, feedback_status, email FROM friend_link WHERE id = $1",
    )
    .persistent(false)
    .bind(id)
    .fetch_optional(pg)
    .await
}

/// 加行锁读取, 目的是让并发审核被串行化 —— 否则两个请求可能各发一封通知邮件。
pub async fn get_for_update(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
) -> sqlx::Result<Option<FriendLinkMailRow>> {
    sqlx::query_as::<_, FriendLinkMailRow>(
        "SELECT site_name, site_url, feedback_status, email FROM friend_link \
         WHERE id = $1 FOR UPDATE",
    )
    .persistent(false)
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn set_status(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
    status: &str,
) -> sqlx::Result<bool> {
    let affected = sqlx::query("UPDATE friend_link SET feedback_status = $2 WHERE id = $1")
        .persistent(false)
        .bind(id)
        .bind(status)
        .execute(&mut **tx)
        .await?
        .rows_affected();
    Ok(affected > 0)
}

#[cfg(test)]
mod tests {
    /// 管理端列表 SQL 绝不能出现 email 列 —— 这个断言是防止将来有人改成 `SELECT *`。
    #[test]
    fn admin_list_sql_never_selects_email() {
        assert!(!super::SELECT_FRIEND.contains("email"));
    }
}
