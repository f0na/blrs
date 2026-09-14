use std::collections::HashMap;

use axum::Json;
use axum::extract::FromRequestParts;
use axum::extract::Query;
use axum::extract::rejection::JsonRejection;
use axum::http::request::Parts;
use serde::Serialize;

use crate::model::err::{AppError, AppResult, ErrCode};

pub mod article;
pub mod friend;
pub mod search;
pub mod site;
pub mod admin;

/// 请求体提取器。用它而不是直接 `Json<T>`, 是为了让 JSON 解析失败也走统一信封 ——
/// axum 默认的 rejection 会返回它自己的那套响应体。
pub type JsonBody<T> = Result<Json<T>, JsonRejection>;

pub fn body<T>(payload: JsonBody<T>) -> AppResult<Json<T>> {
    payload.map_err(|e| AppError::bad_request(format!("请求体不合法: {}", e.body_text())))
}

/// handler 的统一返回类型: 成功是包着信封的 JSON, 失败是 [`AppError`]。
pub type ApiResult<T> = Result<Json<Res<T>>, AppError>;

/// 统一响应信封: `{"code":0,"msg":"ok","data":{...}}`
///
/// `code` 用数字而不是字符串, 前端可以直接 switch, 日志聚合也不会出现歧义。
#[derive(Debug, Serialize)]
pub struct Res<T> {
    pub code: i32,
    pub msg: String,
    pub data: Option<T>,
}

impl<T> Res<T> {
    pub fn ok(data: T) -> Self {
        Self {
            code: ErrCode::Ok.as_i32(),
            msg: "ok".to_string(),
            data: Some(data),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Pager<T> {
    pub page: i64,
    pub page_size: i64,
    pub total: i64,
    pub list: Vec<T>,
}

impl<T> Pager<T> {
    pub fn new(page: i64, page_size: i64, total: i64, list: Vec<T>) -> Self {
        Self {
            page,
            page_size,
            total,
            list,
        }
    }
}

pub const DEFAULT_PAGE_SIZE: i64 = 10;
pub const MAX_PAGE_SIZE: i64 = 50;

/// 分页 + 任意查询参数的提取器。
///
/// 用原始的 `HashMap<String, String>` 而不是 `Query<Struct>`: 后者在参数类型不对时
/// 会返回 axum 自己的 rejection 体, 绕开我们的统一信封。这里全部自己解析, 保证
/// 出错时也是 `{"code":40000,...}`。
#[derive(Debug, Clone)]
pub struct ListQuery(pub HashMap<String, String>);

impl ListQuery {
    pub fn raw(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(|s| s.as_str()).filter(|s| !s.is_empty())
    }

    pub fn page(&self) -> AppResult<i64> {
        match self.raw("page") {
            // 越界页码统一夹到 1, 不算错误; 但非数字必须报错, 不能静默兜底
            None => Ok(1),
            Some(v) => v
                .parse::<i64>()
                .map(|n| n.max(1))
                .map_err(|_| AppError::bad_request("page 必须是整数")),
        }
    }

    pub fn page_size(&self) -> AppResult<i64> {
        match self.raw("page_size") {
            None => Ok(DEFAULT_PAGE_SIZE),
            Some(v) => v
                .parse::<i64>()
                .map(|n| n.clamp(1, MAX_PAGE_SIZE))
                .map_err(|_| AppError::bad_request("page_size 必须是整数")),
        }
    }

    pub fn offset(&self) -> AppResult<i64> {
        let page = self.page()?;
        let page_size = self.page_size()?;
        // page 没有上界, 直接相乘会溢出: release 下绕成负数, Postgres 会以
        // "OFFSET must not be negative" 报错, 于是变成一个莫名其妙的 500。
        page.checked_sub(1)
            .and_then(|p| p.checked_mul(page_size))
            .ok_or_else(|| AppError::bad_request("page 超出可用范围"))
    }

    /// 只接受 "true" / "false" 两个字面量, 其余一律报错。
    pub fn bool(&self, key: &str) -> AppResult<Option<bool>> {
        match self.raw(key) {
            None => Ok(None),
            Some("true") => Ok(Some(true)),
            Some("false") => Ok(Some(false)),
            Some(_) => Err(AppError::bad_request(format!(
                "{key} 只能是 true 或 false"
            ))),
        }
    }
}

impl<S> FromRequestParts<S> for ListQuery
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // 只做 key=value 的原样拆包, 不做百分号解码 —— 用到的参数都是数字和固定字面量。
        let mut map = HashMap::new();
        if let Some(query) = parts.uri.query() {
            for pair in query.split('&') {
                if pair.is_empty() {
                    continue;
                }
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                map.insert(k.to_string(), v.to_string());
            }
        }
        Ok(ListQuery(map))
    }
}

/// 会做百分号解码的查询参数。
///
/// 只有搜索关键词这种"内容由用户决定"的参数需要它: 空格会被编码成 `%20`, 中文更是一串
/// `%E4%B8%AD`, 不解码就等于拿着编码串去搜。其余接口的参数都是数字和固定字面量, 继续
/// 走不做解码的 [`ListQuery`]。
///
/// 用 axum 自带的 `Query` 来做解码而不是自己拆: 它同时处理了 `%XX` 和 `+` 两种形式。
pub struct DecodedQuery(pub ListQuery);

impl std::ops::Deref for DecodedQuery {
    type Target = ListQuery;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<S> FromRequestParts<S> for DecodedQuery
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(map) = Query::<HashMap<String, String>>::from_request_parts(parts, state)
            .await
            .map_err(|e| AppError::bad_request(format!("查询参数不合法: {}", e.body_text())))?;
        Ok(DecodedQuery(ListQuery(map)))
    }
}

#[cfg(test)]
mod tests {
    use axum::extract::FromRequestParts;
    use axum::http::Request;

    use super::{DecodedQuery, ListQuery};

    fn parts_of(uri: &str) -> axum::http::request::Parts {
        Request::builder()
            .uri(uri)
            .body(())
            .unwrap()
            .into_parts()
            .0
    }

    /// 关键词里的空格和中文必须被解码 —— 不解码的话送给 Algolia 的就是 `%E4%B8%AD%E6%96%87`。
    #[tokio::test]
    async fn decoded_query_unescapes_keyword() {
        let mut parts = parts_of("/search?q=%E4%B8%AD%E6%96%87+rust&page=2");
        let q = DecodedQuery::from_request_parts(&mut parts, &()).await.unwrap();

        assert_eq!(q.raw("q"), Some("中文 rust"));
        // 解码之后分页参数照旧能用 (DecodedQuery 能当 ListQuery 使)
        assert_eq!(q.page().unwrap(), 2);
    }

    /// 对照组: `ListQuery` 不解码 —— 这正是关键词不能走它的原因。
    #[tokio::test]
    async fn plain_list_query_leaves_escapes_alone() {
        let mut parts = parts_of("/search?q=%E4%B8%AD%E6%96%87+rust");
        let raw = ListQuery::from_request_parts(&mut parts, &()).await.unwrap();

        assert_eq!(raw.raw("q"), Some("%E4%B8%AD%E6%96%87+rust"));
    }
}
