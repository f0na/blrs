use std::time::Duration;

use axum::http::{HeaderName, HeaderValue, Method, header};
use axum::middleware::from_fn_with_state;
use axum::routing::{get, post};
use axum::Router;
use tower::ServiceBuilder;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;
use vercel_runtime::Error;
use vercel_runtime::axum::VercelLayer;

use blrs::config::{AppConfig, AppState};
use blrs::{db, handler, middleware as mw, service};

#[tokio::main]
async fn main() -> Result<(), Error> {
    init_tracing();

    let cfg = AppConfig::from_env()?;
    tracing::info!("环境变量校验通过");

    let conns = db::Connections::init().await?;
    db::run_migrations().await?;
    conns.check().await;

    let state = AppState::new(conns, cfg);

    // 站点信息注入放在迁移之后: 只有表已经建好、库里又还没填过内容时才需要它。
    // 失败不拦启动 —— 那说明数据库本身有问题, 拦下来只会连管理端一起用不了,
    // 记 ERROR 后让管理员用 POST /admin/site 手动建。
    if let Err(e) = service::site::seed_from_env(&state).await {
        tracing::error!(error = %e, "站点信息注入失败, 公开接口会报\"站点尚未初始化\"");
    }

    let app = ServiceBuilder::new()
        .layer(VercelLayer::new())
        .service(router(state));

    vercel_runtime::run(app).await
}

fn init_tracing() {
    // 注意: 这个 bin target 的名字是 `axum`, 所以本项目的 tracing target 前缀就是
    // `axum::...` (和 axum 这个库本身同前缀)。axum 库自身的日志量极小, 可以接受。
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,axum=debug,tower_http=warn"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .with_span_events(FmtSpan::CLOSE)
        .try_init();
}

fn router(state: AppState) -> Router {
    // --- 公开接口 (BFF) ---
    let public = Router::new()
        .route("/home", get(handler::bff::home_page))
        .route("/about", get(handler::bff::about_page))
        .route("/friends", get(handler::bff::friend_page))
        .route("/friends/apply", post(handler::bff::apply_friend_link))
        .route("/articles/{id}", get(handler::bff::article_page))
        .route("/articles/{id}/like", post(handler::bff::like_article))
        .route("/search", get(handler::search::articles));

    // --- 定时任务 (CRON_SECRET) ---
    let cron = Router::new()
        .route("/cron/cleanup-views", get(handler::cron::cleanup_views))
        .route_layer(from_fn_with_state(state.clone(), mw::cron::require_cron));

    // --- 管理端资源 (JWT) ---
    // 这里只放需要鉴权的路由; `/admin/login` 在外层单独 merge, 所以 route_layer
    // 覆盖不到它 —— route_layer 只作用于本 router 上已注册的路由。
    let admin_guarded = Router::new()
        .route("/me", get(handler::admin::auth::me))
        .route("/password", post(handler::admin::auth::set_password))
        .route(
            "/site",
            get(handler::admin::site::get)
                .post(handler::admin::site::create)
                .put(handler::admin::site::update),
        )
        .route("/articles", get(handler::admin::article::list).post(handler::admin::article::create))
        .route(
            "/articles/{id}",
            get(handler::admin::article::detail)
                .put(handler::admin::article::update)
                .delete(handler::admin::article::soft_delete),
        )
        .route("/articles/{id}/restore", post(handler::admin::article::restore))
        .route("/articles/{id}/hard", axum::routing::delete(handler::admin::article::hard_delete))
        .route("/articles/{id}/views", get(handler::admin::article::list_views))
        .route("/friends", get(handler::admin::friend::list).post(handler::admin::friend::create))
        .route(
            "/friends/{id}",
            axum::routing::put(handler::admin::friend::update)
                .delete(handler::admin::friend::delete),
        )
        .route("/friends/{id}/approve", post(handler::admin::friend::approve))
        .route("/friends/{id}/refuse", post(handler::admin::friend::refuse))
        .route("/socials", get(handler::admin::social::list).post(handler::admin::social::create))
        .route(
            "/socials/{id}",
            axum::routing::put(handler::admin::social::update)
                .delete(handler::admin::social::delete),
        )
        .route(
            "/mail-config",
            get(handler::admin::mail::get)
                .post(handler::admin::mail::create)
                .put(handler::admin::mail::update),
        )
        .route("/blob/token", post(handler::admin::blob::issue_token))
        // 搜索索引: 文章增删改会自动同步, 这两个是用来修复漂移和整份重置的。
        .route("/search/reindex", post(handler::admin::search::reindex))
        .route(
            "/search/index",
            axum::routing::delete(handler::admin::search::clear),
        )
        .route_layer(from_fn_with_state(state.clone(), mw::auth::require_admin));

    let admin = Router::new()
        .route("/login", post(handler::admin::auth::login))
        .merge(admin_guarded);

    // 所有路由都注册完之后再加层 —— axum 的 `layer` 只作用于调用时已存在的路由。
    // 后加的层在外面, 所以下面从内到外的顺序是: 封禁 -> 日志 -> CORS。
    Router::new()
        .merge(public)
        .merge(cron)
        .nest("/admin", admin)
        // 封禁检查包住所有路由: 被封的 IP 连公开接口和 cron 也一并拒绝。
        .layer(from_fn_with_state(state.clone(), mw::ban::ip_ban_check))
        .layer(from_fn_with_state(state.clone(), mw::logging::log_requests))
        // CORS 在最外层, 这样被拒的响应 (403/401) 也带 CORS 头,
        // 前端才能读到状态码而不是只看到一个语焉不详的跨域错误。
        .layer(cors_layer(&state))
        .with_state(state)
}

/// 前端在另一个域名, 所以必须放开 CORS。允许来源由 `CORS_ORIGINS` 配置。
fn cors_layer(state: &AppState) -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        // 管理端靠 Authorization 头带 JWT, 所以这个头必须放行。
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION, header::ACCEPT])
        // 否则前端读不到 x-request-id, 报障时就没法拿它去日志里定位。
        .expose_headers([HeaderName::from_static("x-request-id")])
        .max_age(Duration::from_secs(600));

    if state.cfg.cors_origins.iter().any(|o| o == "*") {
        tracing::warn!("CORS_ORIGINS 配置为 *, 任何来源都可以调用本 API");
        return layer.allow_origin(Any);
    }

    let origins: Vec<HeaderValue> = state
        .cfg
        .cors_origins
        .iter()
        .filter_map(|origin| match HeaderValue::from_str(origin) {
            Ok(value) => Some(value),
            Err(e) => {
                tracing::error!(origin = %origin, error = %e, "CORS_ORIGINS 里有一项不是合法的 header 值, 已忽略");
                None
            }
        })
        .collect();

    tracing::info!(origins = ?state.cfg.cors_origins, "CORS 允许来源已配置");
    layer.allow_origin(AllowOrigin::list(origins))
}
