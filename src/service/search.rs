//! Algolia 搜索索引。
//!
//! 索引里**只放能被搜到的文章**: 已发布且不在回收站。草稿和回收站的文章不是"标记成
//! 不可搜", 而是压根不写进去 —— 这样搜索请求不需要任何 filter, 也就不会因为忘了配
//! `attributesForFaceting` 而搜出草稿。
//!
//! 正文不进索引: `article.content` 列上限 50 万字符 (约 1.5MB), 而 Algolia 单条记录
//! 上限 100KB、整个索引的平均记录大小还必须压在 10KB 以内, 原样推上去会被拒绝。
//! 所以搜得到的是标题、摘要和标签。

use serde::{Deserialize, Serialize};

use crate::config::AppState;
use crate::model::err::{AppError, AppResult};
use crate::model::handler::search::SearchArticle;
use crate::repo;
use crate::repo::article::ArticleListRow;
use crate::util::fmt_ts;

/// 索引名。写死而不是走环境变量: 全站只有一个索引, 改名字等于换一份数据。
const INDEX_NAME: &str = "articles";

/// 一次 batch 推多少条。1000 是 Algolia 官方客户端 (algoliasearch) 的默认值。
const BATCH_SIZE: usize = 1000;

/// 搜索时只取这几个字段, 别把 Algolia 存的所有东西都拖回来。
const RETRIEVE_ATTRIBUTES: &[&str] = &[
    "objectID",
    "title",
    "slug",
    "cover",
    "synopsis",
    "tags",
    "created_at",
    "updated_at",
];

/// 索引里一条文章记录的形态。
#[derive(Debug, Serialize)]
struct ArticleRecord {
    /// 用文章 id 当主键, 写入就天然是按 id 覆盖 —— 重复推送同一篇不会写出两条记录。
    #[serde(rename = "objectID")]
    object_id: String,
    title: String,
    slug: String,
    cover: Option<String>,
    synopsis: Option<String>,
    tags: Vec<String>,
    /// 原始 Unix 秒。不存格式化好的字符串 —— 展示时区只该有一个地方说了算
    /// (`util::DISPLAY_TZ_OFFSET_SECS`), 索引里固化一个时区会让以后换时区变成一次全量重建。
    created_at: i64,
    updated_at: Option<i64>,
}

impl From<&ArticleListRow> for ArticleRecord {
    fn from(row: &ArticleListRow) -> Self {
        Self {
            object_id: row.id.clone(),
            title: row.title.clone(),
            slug: row.slug.clone(),
            cover: row.cover.clone(),
            synopsis: row.synopsis.clone(),
            tags: row.tags.clone(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Algolia 返回的一条命中。
///
/// 除 `objectID` 外全部给了 `#[serde(default)]`: 库里的坏记录应该只让那一条变成空壳,
/// 而不是让整个搜索请求 500。
#[derive(Debug, Deserialize)]
struct SearchHit {
    #[serde(rename = "objectID")]
    object_id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    cover: Option<String>,
    #[serde(default)]
    synopsis: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    created_at: i64,
    #[serde(default)]
    updated_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    hits: Vec<SearchHit>,
    /// 命中总数 (不只是本页的条数)。
    #[serde(rename = "nbHits", default)]
    nb_hits: i64,
}

// ---------------------------------------------------------------------------
// 对外
// ---------------------------------------------------------------------------

/// 搜索。关键词由调用方校验过非空。
pub async fn query(
    st: &AppState,
    keyword: &str,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<SearchArticle>, i64)> {
    let body = serde_json::json!({
        "query": keyword,
        // Algolia 的页码从 0 开始, 对外的分页从 1 开始。
        "page": page - 1,
        "hitsPerPage": page_size,
        "attributesToRetrieve": RETRIEVE_ATTRIBUTES,
    });

    let response = st
        .http
        .post(endpoint(st, "query"))
        .header("x-algolia-application-id", &st.cfg.algolia.app_id)
        // 读路径只用 search key —— 有写权限的那把不出现在这里。
        .header("x-algolia-api-key", &st.cfg.algolia.search_key)
        .json(&body)
        .send()
        .await?;

    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        // 只记状态码和响应体, 绝不记密钥
        tracing::error!(status = status.as_u16(), body = %text, "algolia 搜索失败");
        return Err(AppError::upstream(format!(
            "algolia 搜索失败, HTTP {}",
            status.as_u16()
        )));
    }

    let parsed: SearchResponse = serde_json::from_str(&text).map_err(|e| {
        tracing::error!(error = %e, body_len = text.len(), "algolia 搜索响应无法解析");
        AppError::upstream("algolia 搜索响应格式异常")
    })?;

    let list = parsed
        .hits
        .into_iter()
        .map(|hit| SearchArticle {
            id: hit.object_id,
            title: hit.title,
            slug: hit.slug,
            cover: hit.cover,
            synopsis: hit.synopsis,
            create_at: fmt_ts(hit.created_at),
            update_at: hit.updated_at.map(fmt_ts),
            tags: hit.tags,
        })
        .collect();

    tracing::debug!(keyword, page, page_size, total = parsed.nb_hits, "搜索完成");

    Ok((list, parsed.nb_hits))
}

/// 把一篇文章的当前状态同步到索引。
///
/// **尽力而为**: 内部任何失败都只记日志, 绝不让文章本身的增删改跟着失败。索引和库不同步
/// 时用管理端的全量重建修。
pub async fn sync(st: &AppState, article_id: &str) {
    if let Err(e) = try_sync(st, article_id).await {
        tracing::error!(
            article_id,
            detail = %e.internal,
            "同步搜索索引失败, 这篇文章在搜索里的状态会滞后, 需要时调全量重建"
        );
    }
}

/// 全量重建: 清空索引, 再把当前所有可搜索的文章推上去。
///
/// 清空这一步是为了把"库删了但索引里还留着"的漂移一并清掉 —— 只推不删的话, 那些记录会
/// 一直留在搜索里。代价是中途失败会留下一个不完整的索引, 这时再点一次即可。
pub async fn reindex(st: &AppState) -> AppResult<usize> {
    // 先查库再清空: 反过来写的话, 数据库这一查要是失败了, 索引已经被清掉, 搜索就整个空了。
    let rows = repo::article::list_indexable(&st.conns.pg).await?;
    let records: Vec<ArticleRecord> = rows.iter().map(ArticleRecord::from).collect();

    clear_index(st).await?;

    for chunk in records.chunks(BATCH_SIZE) {
        let requests = chunk
            .iter()
            .map(|record| serde_json::json!({ "action": "addObject", "body": record }))
            .collect();
        send_batch(st, requests).await?;
    }

    Ok(records.len())
}

/// 清空整个索引。记录不会自己回来, 清完必须跟一次全量重建。
pub async fn clear_index(st: &AppState) -> AppResult<()> {
    let response = st
        .http
        .post(endpoint(st, "clear"))
        .header("x-algolia-application-id", &st.cfg.algolia.app_id)
        .header("x-algolia-api-key", &st.cfg.algolia.write_key)
        .json(&serde_json::json!({}))
        .send()
        .await?;

    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        tracing::error!(status = status.as_u16(), body = %text, "algolia 清空索引失败");
        return Err(AppError::upstream(format!(
            "algolia 清空索引失败, HTTP {}",
            status.as_u16()
        )));
    }

    // 这里用 debug 而不是 warn: 全量重建每次都会走到这儿, 用 warn 会把正常的重建
    // 刷成一片告警。真正需要提醒的"索引空了"由管理端 handler 那条 WARN 负责。
    tracing::debug!("algolia 索引已清空");
    Ok(())
}

// ---------------------------------------------------------------------------
// 内部
// ---------------------------------------------------------------------------

async fn try_sync(st: &AppState, article_id: &str) -> AppResult<()> {
    // 查不到行 = 这篇文章不该被搜到: 真删了、在回收站里、或者还是草稿。
    // 三种情况合并成一个判断, 所以增删改、恢复、真删都用同一个调用点, 调用方不用区分动作。
    let Some(row) = repo::article::get_indexable(&st.conns.pg, article_id).await? else {
        tracing::debug!(article_id, "文章不可搜索 (不存在/回收站/草稿), 从索引里删掉");
        return delete_object(st, article_id).await;
    };

    send_batch(
        st,
        vec![serde_json::json!({ "action": "addObject", "body": ArticleRecord::from(&row) })],
    )
    .await
}

async fn delete_object(st: &AppState, object_id: &str) -> AppResult<()> {
    send_batch(
        st,
        vec![serde_json::json!({
            "action": "deleteObject",
            "body": { "objectID": object_id },
        })],
    )
    .await
}

/// 提交一批写操作。调用方负责分批。
async fn send_batch(st: &AppState, requests: Vec<serde_json::Value>) -> AppResult<()> {
    if requests.is_empty() {
        return Ok(());
    }

    let count = requests.len();
    let response = st
        .http
        .post(endpoint(st, "batch"))
        .header("x-algolia-application-id", &st.cfg.algolia.app_id)
        .header("x-algolia-api-key", &st.cfg.algolia.write_key)
        .json(&serde_json::json!({ "requests": requests }))
        .send()
        .await?;

    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        // 只记状态码和响应体, 绝不记密钥
        tracing::error!(status = status.as_u16(), count, body = %text, "algolia 写入失败");
        return Err(AppError::upstream(format!(
            "algolia 写入失败, HTTP {}",
            status.as_u16()
        )));
    }

    tracing::debug!(count, "algolia 写入完成");
    Ok(())
}

/// 索引接口的地址。用 `-dsn` 子域是 Algolia 的推荐入口 (DSN = Distributed Search
/// Network), 读写都走它, 不用自己拼 `-1/-2/-3.algolianet.com` 那串节点。
fn endpoint(st: &AppState, action: &str) -> String {
    format!(
        "https://{}-dsn.algolia.net/1/indexes/{INDEX_NAME}/{action}",
        st.cfg.algolia.app_id
    )
}

#[cfg(test)]
mod tests {
    use super::{ArticleRecord, SearchResponse};
    use crate::repo::article::ArticleListRow;

    fn row() -> ArticleListRow {
        ArticleListRow {
            id: "0192".to_string(),
            title: "标题".to_string(),
            slug: "slug".to_string(),
            cover: None,
            synopsis: Some("摘要".to_string()),
            likes: 3,
            status: "Pub".to_string(),
            created_at: 1_700_000_000,
            updated_at: None,
            views: 9,
            tags: vec!["rust".to_string()],
        }
    }

    /// 索引记录的主键必须是 `objectID`, 且只带搜索要用的字段 —— 正文、点赞、浏览量
    /// 都不该出现在推给 Algolia 的 JSON 里。
    #[test]
    fn record_uses_object_id_and_omits_dynamic_fields() {
        let json = serde_json::to_value(ArticleRecord::from(&row())).unwrap();
        assert_eq!(json["objectID"], "0192");
        assert_eq!(json["tags"][0], "rust");
        for absent in ["content", "likes", "views", "status", "deleted_at"] {
            assert!(json.get(absent).is_none(), "{absent} 不该进索引");
        }
    }

    /// 索引里的记录坏掉时, 只该让那一条空掉, 不该让整个搜索请求失败。
    #[test]
    fn malformed_hit_does_not_fail_the_whole_response() {
        let body = r#"{"hits":[{"objectID":"a"},{"objectID":"b","title":"t"}],"nbHits":2}"#;
        let parsed: SearchResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.nb_hits, 2);
        assert_eq!(parsed.hits.len(), 2);
        assert!(parsed.hits[0].title.is_empty());
    }
}
