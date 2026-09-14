use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};

use crate::model::err::AppResult;
use crate::util::now_secs;

/// `type` 是 SQL 关键字, 列名在 DDL 里带引号, 这里统一别名成 `social_type`。
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct SocialRow {
    pub id: String,
    pub social_type: String,
    pub icon: Option<String>,
    pub value: String,
    pub created_at: i64,
}

const SELECT_SOCIAL: &str = "SELECT id, \"type\" AS social_type, icon, value, created_at FROM social";

pub async fn list(pg: &PgPool) -> AppResult<Vec<SocialRow>> {
    let sql = format!("{SELECT_SOCIAL} ORDER BY created_at ASC, id ASC");
    Ok(sqlx::query_as::<_, SocialRow>(sqlx::AssertSqlSafe(sql))
        .persistent(false)
        .fetch_all(pg)
        .await?)
}

pub async fn create(pg: &PgPool, id: &str, social_type: &str, icon: Option<&str>, value: &str) -> AppResult<()> {
    sqlx::query("INSERT INTO social (id, \"type\", icon, value, created_at) VALUES ($1, $2, $3, $4, $5)")
        .persistent(false)
        .bind(id)
        .bind(social_type)
        .bind(icon)
        .bind(value)
        .bind(now_secs())
        .execute(pg)
        .await?;
    Ok(())
}

pub async fn update(pg: &PgPool, id: &str, social_type: &str, icon: Option<&str>, value: &str) -> AppResult<bool> {
    let affected = sqlx::query("UPDATE social SET \"type\" = $2, icon = $3, value = $4 WHERE id = $1")
        .persistent(false)
        .bind(id)
        .bind(social_type)
        .bind(icon)
        .bind(value)
        .execute(pg)
        .await?
        .rows_affected();
    Ok(affected > 0)
}

pub async fn delete(pg: &PgPool, id: &str) -> AppResult<bool> {
    let affected = sqlx::query("DELETE FROM social WHERE id = $1")
        .persistent(false)
        .bind(id)
        .execute(pg)
        .await?
        .rows_affected();
    Ok(affected > 0)
}
