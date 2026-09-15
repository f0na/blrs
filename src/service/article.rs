use crate::config::{AppState, VIEW_DEDUPE_SECS};
use crate::repo;
use crate::util::now_secs;

/// 对齐 `article_tag.tag_content VARCHAR(100)`。
const MAX_TAG_LEN: usize = 100;
const MAX_TAGS: usize = 15;

const VIEW_KEY_PREFIX: &str = "blrs:view:";
const LIKE_KEY_PREFIX: &str = "blrs:like:";

/// 收敛标签入参: 去空白、去空串、去重、截断到列宽、限制个数。
///
/// 不这么做的话, 一个超过 100 字符的标签会让 INSERT 直接报错, 对整个请求来说
/// 就是一个莫名其妙的 500。
pub fn sanitize_tags(tags: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(tags.len());
    for tag in tags {
        let trimmed = tag.trim();
        if trimmed.is_empty() {
            continue;
        }
        let cut: String = trimmed.chars().take(MAX_TAG_LEN).collect();
        if out.iter().any(|existing| existing == &cut) {
            continue;
        }
        out.push(cut);
        if out.len() >= MAX_TAGS {
            break;
        }
    }
    out
}

/// `SET NX EX` 抢一个计数窗口, 返回 `true` 表示是我们抢到的 —— 这个窗口内还没计过。
///
/// Valkey 没配或查询失败时一律返回 `true` (照常计数)。缓存故障只该让计数偏高,
/// 不该把浏览和点赞一起丢掉。
async fn claim_window(st: &AppState, key: &str) -> bool {
    let Some(mut conn) = st.conns.valkey() else {
        return true;
    };
    match redis::cmd("SET")
        .arg(key)
        .arg("1")
        .arg("NX")
        .arg("EX")
        .arg(VIEW_DEDUPE_SECS)
        .query_async::<Option<String>>(&mut conn)
        .await
    {
        // SET NX 返回 OK 说明是我们抢到的, 这个窗口内还没计过
        Ok(Some(_)) => true,
        // 返回 nil 说明 key 已存在, 窗口内已经计过
        Ok(None) => false,
        Err(e) => {
            tracing::warn!(error = %e, key, "valkey 去重失败, 本次直接计数");
            true
        }
    }
}

/// 记录一次浏览。
///
/// 同一 IP 在 1 小时窗口内只计一次, 去重状态放 Valkey。
/// 这是**尽力而为**的旁路: 无论 Valkey 还是数据库出问题都只记日志, 绝不让详情页失败。
pub async fn record_view(st: &AppState, article_id: &str, client_ip: &str) {
    if !claim_window(st, &format!("{VIEW_KEY_PREFIX}{article_id}:{client_ip}")).await {
        tracing::debug!(article_id, "同一 IP 在 1 小时窗口内已计过, 跳过");
        return;
    }

    match repo::article::insert_view(&st.conns.pg, article_id, now_secs()).await {
        Ok(()) => tracing::debug!(article_id, "浏览记录已写入"),
        Err(e) => tracing::warn!(error = %e, article_id, "浏览记录写入失败"),
    }
}

/// 点赞。同一 IP 在同一个窗口内只累加一次, 去重规则和 [`record_view`] 完全一致。
///
/// 返回文章最新的点赞数; `None` 表示文章不存在或未发布, 由调用方转成 404。
/// 窗口内重复点赞**不报错也不累加**, 就是把当前值原样返回 —— 前端拿到的响应结构不变。
/// 和浏览一样是尽力而为的防刷: 伪造不了 IP 的人刷不动, 肯换 IP 的仍然能刷。
pub async fn record_like(
    st: &AppState,
    article_id: &str,
    client_ip: &str,
) -> sqlx::Result<Option<i64>> {
    if !claim_window(st, &format!("{LIKE_KEY_PREFIX}{article_id}:{client_ip}")).await {
        tracing::debug!(article_id, "同一 IP 在窗口内已点过赞, 不再累加");
        return repo::article::likes_of(&st.conns.pg, article_id).await;
    }

    repo::article::like(&st.conns.pg, article_id).await
}

#[cfg(test)]
mod tests {
    use super::sanitize_tags;

    #[test]
    fn sanitize_tags_dedupes_and_trims() {
        let tags = vec!["  rust ".to_string(), "rust".to_string(), "".to_string(), "axum".to_string()];
        assert_eq!(sanitize_tags(tags), vec!["rust", "axum"]);
    }

    #[test]
    fn sanitize_tags_truncates_to_column_width() {
        let long = "字".repeat(200);
        let out = sanitize_tags(vec![long]);
        assert_eq!(out[0].chars().count(), 100);
    }
}
