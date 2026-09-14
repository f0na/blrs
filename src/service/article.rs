use crate::config::{AppState, VIEW_DEDUPE_SECS};
use crate::repo;
use crate::util::now_secs;

/// 对齐 `article_tag.tag_content VARCHAR(100)`。
const MAX_TAG_LEN: usize = 100;
const MAX_TAGS: usize = 15;

const VIEW_KEY_PREFIX: &str = "blrs:view:";

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

/// 记录一次浏览。
///
/// 同一 IP 在 1 小时窗口内只计一次, 去重状态放 Valkey。
/// 这是**尽力而为**的旁路: 无论 Valkey 还是数据库出问题都只记日志, 绝不让详情页失败。
pub async fn record_view(st: &AppState, article_id: &str, client_ip: &str) {
    let key = format!("{VIEW_KEY_PREFIX}{article_id}:{client_ip}");

    let first_in_window = match st.conns.valkey() {
        None => true,
        Some(mut conn) => match redis::cmd("SET")
            .arg(&key)
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
                // Valkey 挂了就退化成"每次请求都记一条"。计数会偏高, 但不会报错,
                // 也不会因为缓存故障丢掉浏览量。
                tracing::warn!(error = %e, article_id, "valkey 去重失败, 本次浏览将直接入库");
                true
            }
        },
    };

    if !first_in_window {
        tracing::debug!(article_id, "同一 IP 在 1 小时窗口内已计过, 跳过");
        return;
    }

    match repo::article::insert_view(&st.conns.pg, article_id, now_secs()).await {
        Ok(()) => tracing::debug!(article_id, "浏览记录已写入"),
        Err(e) => tracing::warn!(error = %e, article_id, "浏览记录写入失败"),
    }
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
