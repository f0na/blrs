use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::config::AppState;
use crate::model::err::AppError;
use crate::service;
use crate::util::trusted_client_ip;

/// 全局 IP 封禁检查。这一层在**最外面**, 所以被封的 IP 访问任何接口
/// (公开的、管理端的、cron 的) 都会被拦下。
pub async fn ip_ban_check(State(st): State<AppState>, req: Request, next: Next) -> Response {
    let Some(ip) = trusted_client_ip(req.headers()) else {
        // 拿不到可信来源就不做封禁判断。宁可漏封, 也不能因为一个客户端能伪造的头
        // 把任意 IP 封掉 90 天。本地开发没有这些头, 所以只记 debug 避免刷屏。
        tracing::debug!("无法确定可信客户端 IP, 跳过封禁检查");
        return next.run(req).await;
    };

    if service::ban::is_banned(&st, &ip).await {
        tracing::debug!(ip = %ip, path = %req.uri().path(), "已封禁的 IP 请求被拒绝");
        return AppError::forbidden("forbidden").into_response();
    }

    next.run(req).await
}
