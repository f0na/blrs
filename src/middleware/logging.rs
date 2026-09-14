use std::time::Instant;

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::Response;
use tracing::Instrument;

use crate::config::AppState;
use crate::util::client_ip;

/// 超过这个耗时额外打一条 WARN, 便于把慢请求 grep 出来。
const SLOW_REQUEST_MS: u128 = 1000;

const REQUEST_ID_HEADER: &str = "x-request-id";

/// 详细访问日志。
///
/// 整个下游调用都在 `http` span 里执行, 所以 handler 和错误日志会自动带上
/// request_id / method / path / client_ip, 不需要每个地方手动传。
pub async fn log_requests(State(_st): State<AppState>, req: Request, next: Next) -> Response {
    let started = Instant::now();

    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or_default().to_string();
    let ip = client_ip(req.headers());

    // 客户端能否伪造这些头都无所谓 —— 它们只进日志, 不参与任何决策。
    let user_agent = header(req.headers(), header::USER_AGENT.as_str());
    let referer = header(req.headers(), header::REFERER.as_str());
    let content_length = header(req.headers(), header::CONTENT_LENGTH.as_str());
    let vercel_id = header(req.headers(), "x-vercel-id");

    // Authorization / Cookie 只记录是否存在, 绝不记录原值。
    let has_authorization = req.headers().contains_key(header::AUTHORIZATION);
    let has_cookie = req.headers().contains_key(header::COOKIE);

    let request_id = req
        .headers()
        .get(REQUEST_ID_HEADER)
        .or_else(|| req.headers().get("x-vercel-id"))
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::now_v7().to_string());

    let span = tracing::info_span!(
        "http",
        request_id = %request_id,
        method = %req.method(),
        path = %path,
        client_ip = %ip,
    );

    let request_id_for_response = request_id.clone();

    async move {
        tracing::debug!(
            query = %query,
            user_agent = %user_agent,
            referer = %referer,
            content_length = %content_length,
            x_vercel_id = %vercel_id,
            has_authorization,
            has_cookie,
            "请求开始"
        );

        let mut response = next.run(req).await;

        let status = response.status();
        let latency_ms = started.elapsed().as_millis();
        let status_code = status.as_u16();

        // 注意: vercel_runtime 把 application/json 也当成流式响应走 mpsc 泵,
        // 所以这里不打印响应体字节数, 需要的话看请求的 content-length。
        if status.is_server_error() {
            tracing::error!(status = status_code, latency_ms, "请求完成");
        } else if status.is_client_error() {
            tracing::warn!(status = status_code, latency_ms, "请求完成");
        } else {
            tracing::info!(status = status_code, latency_ms, "请求完成");
        }

        if latency_ms > SLOW_REQUEST_MS {
            tracing::warn!(status = status_code, latency_ms, threshold_ms = SLOW_REQUEST_MS, "慢请求");
        }

        if let Ok(value) = axum::http::HeaderValue::from_str(&request_id_for_response) {
            response.headers_mut().insert(REQUEST_ID_HEADER, value);
        }

        response
    }
    .instrument(span)
    .await
}

fn header(headers: &axum::http::HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}
