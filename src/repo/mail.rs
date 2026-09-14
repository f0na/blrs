use sqlx::{FromRow, PgPool};

use crate::model::err::{AppError, AppResult};
use crate::util::now_secs;

/// 完整的一行, 含 SMTP 密码。只在真正要发信时读取。
#[derive(Debug, Clone, FromRow)]
pub struct MailConfigRow {
    pub id: String,
    pub host: String,
    pub port: i32,
    pub username: String,
    pub password: String,
    pub from_addr: String,
}

/// 管理端读的那一份: **不含密码**, 只带一个"有没有配过"的布尔。
#[derive(Debug, Clone, FromRow)]
pub struct MailConfigAdminRow {
    pub id: String,
    pub host: String,
    pub port: i32,
    pub username: String,
    pub from_addr: String,
    pub enable: bool,
    pub has_password: bool,
    pub created_at: i64,
}

const SELECT_ADMIN: &str = "SELECT id, host, port, username, from_addr, enable, \
                            (password <> '') AS has_password, created_at FROM mail_config";

/// 唯一启用的一条。有多条时取最新创建的那条。
pub async fn get_enabled(pg: &PgPool) -> AppResult<Option<MailConfigRow>> {
    let rows = sqlx::query_as::<_, MailConfigRow>(
        "SELECT id, host, port, username, password, from_addr FROM mail_config \
         WHERE enable = TRUE ORDER BY created_at DESC",
    )
    .persistent(false)
    .fetch_all(pg)
    .await?;
    if rows.len() > 1 {
        tracing::warn!(
            count = rows.len(),
            "mail_config 里有多条 enable = TRUE, 取最新的一条"
        );
    }
    Ok(rows.into_iter().next())
}

pub async fn get_admin(pg: &PgPool) -> AppResult<Option<MailConfigAdminRow>> {
    let sql = format!("{SELECT_ADMIN} ORDER BY created_at DESC LIMIT 1");
    Ok(sqlx::query_as::<_, MailConfigAdminRow>(sqlx::AssertSqlSafe(sql))
        .persistent(false)
        .fetch_optional(pg)
        .await?)
}

/// 邮件配置的写入字段。`password` 为 None 在更新时表示保持原密码不变。
pub struct MailFields<'a> {
    pub host: &'a str,
    pub port: i32,
    pub username: &'a str,
    pub password: Option<&'a str>,
    pub from_addr: &'a str,
    pub enable: bool,
}

pub async fn create(pg: &PgPool, id: &str, fields: &MailFields<'_>) -> AppResult<()> {
    let password = fields
        .password
        .ok_or_else(|| AppError::bad_request("新建邮件配置时必须提供 SMTP 密码"))?;

    sqlx::query(
        "INSERT INTO mail_config (id, host, port, username, password, from_addr, enable, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .persistent(false)
    .bind(id)
    .bind(fields.host)
    .bind(fields.port)
    .bind(fields.username)
    .bind(password)
    .bind(fields.from_addr)
    .bind(fields.enable)
    .bind(now_secs())
    .execute(pg)
    .await?;
    Ok(())
}

pub async fn update(pg: &PgPool, id: &str, fields: &MailFields<'_>) -> AppResult<bool> {
    let affected = sqlx::query(
        "UPDATE mail_config SET host = $2, port = $3, username = $4, \
         password = COALESCE($5, password), from_addr = $6, enable = $7 WHERE id = $1",
    )
    .persistent(false)
    .bind(id)
    .bind(fields.host)
    .bind(fields.port)
    .bind(fields.username)
    .bind(fields.password)
    .bind(fields.from_addr)
    .bind(fields.enable)
    .execute(pg)
    .await?
    .rows_affected();
    Ok(affected > 0)
}
