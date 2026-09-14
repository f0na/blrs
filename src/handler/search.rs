use axum::Json;
use axum::extract::State;
use tracing::instrument;

use crate::config::AppState;
use crate::model::err::AppResult;
use crate::model::handler::search::SearchArticle;
use crate::model::handler::{ApiResult, DecodedQuery, Pager, Res};
use crate::service;
use crate::util::required_text;

/// 关键词长度上限。Algolia 用不着更长, 而查询串是会被原样转发出去的。
const MAX_KEYWORD_LEN: usize = 100;

/// 搜索文章。函数名不叫 `page` 是因为它**不是**页面接口, 见下。
///
/// **刻意不套 `{site, social, ...}` 那层页面信封** —— 和 `/home`、`/friends` 那几个不一样,
/// 这个接口服务的是搜索框下拉: 站点信息调用方本来就有, 每次敲键都带一份是纯浪费, 而且
/// 它还得为此多查一次缓存和一次库。返回的就是一个 [`Pager`], 和 `/admin/articles` 同一种形状。
///
/// 参数用 [`DecodedQuery`] 而不是普通的 `ListQuery`: 关键词里有空格和中文, 必须解码。
#[instrument(skip_all)]
pub async fn articles(
    State(st): State<AppState>,
    q: DecodedQuery,
) -> ApiResult<Pager<SearchArticle>> {
    let (page, page_size) = (q.page()?, q.page_size()?);

    // 空关键词给空结果而不是报错 (见 resolve_keyword)。
    let Some(keyword) = resolve_keyword(q.raw("q"))? else {
        return Ok(Json(Res::ok(Pager::new(page, page_size, 0, Vec::new()))));
    };

    let (list, total) = service::search::query(&st, &keyword, page, page_size).await?;

    Ok(Json(Res::ok(Pager::new(page, page_size, total, list))))
}

/// 收敛关键词。返回 `None` 表示"没什么可搜的", 调用方据此给一个空结果。
///
/// 空输入**不是**参数错误: 搜索框长在每一页上, 被清空是它的常态。要是这里返回 40000,
/// 前端就得为了"别把空串送出去"多加一个分支, 日志里也会被这类请求刷满。
/// 真正的错误只有超长 —— 那才是真的写错了。
fn resolve_keyword(raw: Option<&str>) -> AppResult<Option<String>> {
    match raw.map(str::trim) {
        None | Some("") => Ok(None),
        Some(keyword) => required_text(keyword, "搜索关键词", MAX_KEYWORD_LEN).map(Some),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_keyword;

    #[test]
    fn empty_keyword_is_not_an_error() {
        assert_eq!(resolve_keyword(None).unwrap(), None);
        assert_eq!(resolve_keyword(Some("")).unwrap(), None);
        assert_eq!(resolve_keyword(Some("   ")).unwrap(), None);
    }

    #[test]
    fn keyword_is_trimmed_and_bounded() {
        assert_eq!(resolve_keyword(Some("  rust  ")).unwrap().as_deref(), Some("rust"));

        // 按字符数算, 中文不会因为占了 3 个字节被误判超长
        let at_limit = "字".repeat(100);
        assert!(resolve_keyword(Some(&at_limit)).is_ok());

        let over_limit = "字".repeat(101);
        assert!(resolve_keyword(Some(&over_limit)).is_err());
    }
}
