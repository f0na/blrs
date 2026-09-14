use argon2::password_hash::phc::PasswordHash;
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use crate::config::{AppConfig, AppState, JWT_TTL_SECS};
use crate::model::err::{AppError, AppResult};
use crate::repo;
use crate::util::now_secs;

/// 密码最短长度。允许空密码就等于把 `site_secret` 清空 = 谁都能登录。
const MIN_PASSWORD_LEN: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminClaims {
    pub sub: String,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
}

pub fn hash_password(pswd: &str) -> AppResult<String> {
    Argon2::default()
        .hash_password(pswd.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|e| AppError::internal(format!("密码散列失败: {e}")))
}

/// 库里是否已经设置过密码。
pub async fn password_is_set(st: &AppState) -> AppResult<bool> {
    Ok(repo::site::password_hash(&st.conns.pg).await?.is_some())
}

/// 校验登录密码。
///
/// 尚未设置密码时返回 true —— 这是明确要求的行为: 首次登录不需要密码, 之后再设置。
/// 这个窗口是公开的, 所以这里会打一条 WARN 提醒。
pub async fn verify_login(st: &AppState, pswd: Option<&str>) -> AppResult<bool> {
    let Some(hash) = repo::site::password_hash(&st.conns.pg).await? else {
        tracing::warn!("管理员密码尚未设置, 本次登录免密放行 —— 请尽快调用 POST /admin/password 设置");
        return Ok(true);
    };
    Ok(verify_password(pswd.unwrap_or(""), &hash))
}

pub fn verify_password(pswd: &str, hash: &str) -> bool {
    match PasswordHash::new(hash) {
        // verify_password 内部是常量时间比较
        Ok(parsed) => Argon2::default()
            .verify_password(pswd.as_bytes(), &parsed)
            .is_ok(),
        Err(e) => {
            tracing::error!(error = %e, "库里的密码散列不是合法 PHC 串, 无法校验");
            false
        }
    }
}

pub async fn set_password(st: &AppState, pswd: &str) -> AppResult<()> {
    if pswd.chars().count() < MIN_PASSWORD_LEN {
        return Err(AppError::bad_request(format!(
            "密码至少 {MIN_PASSWORD_LEN} 位"
        )));
    }
    let hash = hash_password(pswd)?;
    repo::site::set_password_hash(&st.conns.pg, &hash).await?;
    tracing::info!("管理员密码已更新");
    Ok(())
}

/// 签发 JWT, 返回 `(token, 过期时间戳)`。
pub fn issue_token(cfg: &AppConfig) -> AppResult<(String, i64)> {
    let now = now_secs();
    let exp = now + JWT_TTL_SECS;
    let claims = AdminClaims {
        sub: "admin".to_string(),
        iat: now,
        exp,
        jti: uuid::Uuid::now_v7().to_string(),
    };
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(cfg.jwt_secret.as_bytes()),
    )?;
    Ok((token, exp))
}

pub fn verify_token(cfg: &AppConfig, token: &str) -> AppResult<AdminClaims> {
    let data = decode::<AdminClaims>(
        token,
        &DecodingKey::from_secret(cfg.jwt_secret.as_bytes()),
        &Validation::new(Algorithm::HS256),
    )?;
    Ok(data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_roundtrip() {
        let hash = hash_password("hunter2!").unwrap();
        assert!(verify_password("hunter2!", &hash));
        assert!(!verify_password("hunter3!", &hash));
    }

    #[test]
    fn malformed_hash_never_panics() {
        assert!(!verify_password("x", "not-a-phc-string"));
    }
}
