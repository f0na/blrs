use crate::config::{AppState, BAN_SECS};

const BAN_PREFIX: &str = "blrs:ban:";
const FAIL_PREFIX: &str = "blrs:login_fail:";

/// 这个 IP 是否处于封禁中。
///
/// Valkey 不可用时返回 false (放行) 并记 WARN —— 缓存故障绝不能把自己锁在门外。
/// 代价是 Valkey 挂掉期间这层防护失效。
pub async fn is_banned(st: &AppState, ip: &str) -> bool {
    let Some(mut conn) = st.conns.valkey() else {
        tracing::warn!(ip, "valkey 不可用, 封禁检查已跳过");
        return false;
    };
    match redis::cmd("EXISTS")
        .arg(format!("{BAN_PREFIX}{ip}"))
        .query_async::<i64>(&mut conn)
        .await
    {
        Ok(count) => count > 0,
        Err(e) => {
            tracing::warn!(error = %e, ip, "封禁状态查询失败, 本次按未封禁处理");
            false
        }
    }
}

/// 记一次登录失败, 返回累计失败次数。
///
/// 计数器**没有 TTL**: 需求是"错 3 次即封 90 天", 计次若会过期, 攻击者只要把节奏
/// 放慢到每个窗口 2 次就能永远绕过封禁。只有登录成功才会清零。
/// 代价是管理员自己累计输错 3 次也会被锁 —— 解封见 README。
/// 返回 None 表示计数本身失败 (Valkey 不可用), 调用方应当放行。
pub async fn record_failure(st: &AppState, ip: &str) -> Option<i64> {
    let Some(mut conn) = st.conns.valkey() else {
        tracing::warn!(ip, "valkey 不可用, 本次登录失败未计数");
        return None;
    };

    match redis::cmd("INCR")
        .arg(format!("{FAIL_PREFIX}{ip}"))
        .query_async::<i64>(&mut conn)
        .await
    {
        Ok(count) => Some(count),
        Err(e) => {
            tracing::warn!(error = %e, ip, "登录失败计数失败, 本次不计数");
            None
        }
    }
}

pub async fn ban(st: &AppState, ip: &str) {
    let Some(mut conn) = st.conns.valkey() else {
        tracing::error!(ip, "valkey 不可用, 封禁未能写入 —— 该 IP 不会被拦截");
        return;
    };
    match redis::cmd("SETEX")
        .arg(format!("{BAN_PREFIX}{ip}"))
        .arg(BAN_SECS)
        .arg("1")
        .query_async::<redis::Value>(&mut conn)
        .await
    {
        Ok(_) => tracing::warn!(
            ip,
            seconds = BAN_SECS,
            days = BAN_SECS / 86400,
            "IP 已封禁"
        ),
        Err(e) => tracing::error!(error = %e, ip, "IP 封禁写入失败"),
    }
}

/// 登录成功后清掉失败计数。
pub async fn clear_failures(st: &AppState, ip: &str) {
    let Some(mut conn) = st.conns.valkey() else {
        return;
    };
    if let Err(e) = redis::cmd("DEL")
        .arg(format!("{FAIL_PREFIX}{ip}"))
        .query_async::<redis::Value>(&mut conn)
        .await
    {
        tracing::warn!(error = %e, ip, "登录失败计数清除失败");
    }
}
