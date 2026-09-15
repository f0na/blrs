use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use serde::Serialize;
use tracing::instrument;

use crate::config::AppState;
use crate::model::LinkStatus;
use crate::model::err::AppError;
use crate::model::handler::{
    ApiResult, JsonBody, ListQuery, Pager, Res, body,
    article::{ArticleDetail, HomeArticleList},
    friend::{FriendApplyInput, FriendLink},
    site::{AboutSite, HomeSite, HomeSocial},
};
use crate::repo;
use crate::service;
use crate::util::{
    optional_text, required_text, trusted_client_ip, validate_email, validate_site_url,
};

/// 把数据库行转成公开模型。标签和浏览量都已经在 SQL 里聚合好。
fn to_article_list(row: &repo::article::ArticleListRow) -> HomeArticleList {
    HomeArticleList {
        id: row.id.clone(),
        title: row.title.clone(),
        slug: row.slug.clone(),
        cover: row.cover.clone(),
        synopsis: row.synopsis.clone(),
        likes: row.likes,
        views: row.views,
        create_at: row.created_at,
        update_at: row.updated_at,
        tags: row.tags.clone(),
    }
}

/// 详情页去重用的客户端标识。
///
/// 用可信来源而不是可伪造的 `x-forwarded-for` 首段: 否则轮换一下头就能把浏览量刷上去。
/// 取不到可信来源时全部归到同一个 key, 相当于"这类请求 1 小时内只计一次"。
fn view_identity(headers: &HeaderMap) -> String {
    trusted_client_ip(headers).unwrap_or_else(|| "-".to_string())
}

#[derive(Debug, Serialize)]
pub struct HomeRes {
    site: HomeSite,
    social: Vec<HomeSocial>,
    article_list: Pager<HomeArticleList>,
}

#[instrument(skip_all)]
pub async fn home_page(State(st): State<AppState>, q: ListQuery) -> ApiResult<HomeRes> {
    let (page, page_size, offset) = (q.page()?, q.page_size()?, q.offset()?);

    // 站点上下文和文章列表互不依赖, 并发取。
    let (ctx, rows, total) = tokio::try_join!(
        service::site::load(&st),
        async { repo::article::list_public(&st.conns.pg, page_size, offset).await.map_err(AppError::from) },
        async { repo::article::count_public(&st.conns.pg).await.map_err(AppError::from) },
    )?;

    tracing::debug!(page, page_size, total, "首页数据已组装");

    Ok(Json(Res::ok(HomeRes {
        site: ctx.home_site(),
        social: ctx.social(),
        article_list: Pager::new(
            page,
            page_size,
            total,
            rows.iter().map(to_article_list).collect(),
        ),
    })))
}

#[derive(Debug, Serialize)]
pub struct AboutRes {
    site: AboutSite,
    social: Vec<HomeSocial>,
}

#[instrument(skip_all)]
pub async fn about_page(State(st): State<AppState>) -> ApiResult<AboutRes> {
    let ctx = service::site::load(&st).await?;
    Ok(Json(Res::ok(AboutRes {
        site: ctx.about_site(),
        social: ctx.social(),
    })))
}

#[derive(Debug, Serialize)]
pub struct FriendRes {
    site: HomeSite,
    social: Vec<HomeSocial>,
    friend_list: Pager<FriendLink>,
}

#[instrument(skip_all)]
pub async fn friend_page(State(st): State<AppState>, q: ListQuery) -> ApiResult<FriendRes> {
    let (page, page_size, offset) = (q.page()?, q.page_size()?, q.offset()?);

    let (ctx, rows, total) = tokio::try_join!(
        service::site::load(&st),
        async { repo::friend::list_public(&st.conns.pg, page_size, offset).await.map_err(AppError::from) },
        async { repo::friend::count_public(&st.conns.pg).await.map_err(AppError::from) },
    )?;

    let list = rows
        .iter()
        .map(|row| FriendLink {
            url: row.site_url.clone(),
            name: row.site_name.clone(),
            intro: row.site_intro.clone(),
            icon: row.site_icon.clone(),
        })
        .collect();

    Ok(Json(Res::ok(FriendRes {
        site: ctx.home_site(),
        social: ctx.social(),
        friend_list: Pager::new(page, page_size, total, list),
    })))
}

#[derive(Debug, Serialize)]
pub struct ArticleRes {
    site: HomeSite,
    social: Vec<HomeSocial>,
    article: ArticleDetail,
}

/// 详情页拿到行之后的公共部分: 记一次浏览, 再组装响应。
/// 按 id 和按 slug 两个入口只有"怎么取行"不同, 到这里就合流了。
async fn build_article_res(
    st: &AppState,
    ctx: service::site::SiteContext,
    row: repo::article::ArticleDetailRow,
    headers: &HeaderMap,
) -> ArticleRes {
    // 计数是旁路: 内部任何失败都只记日志, 详情页照常返回。
    service::article::record_view(st, &row.id, &view_identity(headers)).await;

    ArticleRes {
        site: ctx.home_site(),
        social: ctx.social(),
        article: ArticleDetail {
            id: row.id,
            title: row.title,
            slug: row.slug,
            cover: row.cover,
            synopsis: row.synopsis,
            content: row.content,
            likes: row.likes,
            views: row.views,
            create_at: row.created_at,
            update_at: row.updated_at,
            tags: row.tags,
        },
    }
}

#[instrument(skip_all, fields(article_id = %id))]
pub async fn article_page(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<ArticleRes> {
    let (ctx, row) = tokio::try_join!(
        service::site::load(&st),
        async { repo::article::detail_public(&st.conns.pg, &id).await.map_err(AppError::from) },
    )?;

    let Some(row) = row else {
        return Err(AppError::not_found("文章不存在"));
    };

    Ok(Json(Res::ok(build_article_res(&st, ctx, row, &headers).await)))
}

/// 同一篇文章, 入口改成 slug —— 前端文章页的 URL 就是 `/<slug>`,
/// 有这个入口才不用先拿 id 再换一次。
#[instrument(skip_all, fields(article_slug = %slug))]
pub async fn article_page_by_slug(
    State(st): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
) -> ApiResult<ArticleRes> {
    let (ctx, row) = tokio::try_join!(
        service::site::load(&st),
        async {
            repo::article::detail_public_by_slug(&st.conns.pg, &slug)
                .await
                .map_err(AppError::from)
        },
    )?;

    let Some(row) = row else {
        return Err(AppError::not_found("文章不存在"));
    };

    Ok(Json(Res::ok(build_article_res(&st, ctx, row, &headers).await)))
}

/// 访客提交友链申请, 落库为待审核。
#[instrument(skip_all)]
pub async fn apply_friend_link(
    State(st): State<AppState>,
    payload: JsonBody<FriendApplyInput>,
) -> ApiResult<()> {
    let Json(input) = body(payload)?;

    // 校验后的值要先绑定再取引用 —— 直接在结构体字面量里 `.as_deref()` 会借用临时值。
    let url = validate_site_url(&input.url)?;
    let name = required_text(&input.name, "站点名称", 255)?;
    let intro = required_text(&input.intro, "站点介绍", 500)?;
    let icon = optional_text(input.icon.as_deref(), "站点图标", 500)?;
    let email = validate_email(input.email.as_deref())?;

    let new_link = repo::friend::NewFriendLink {
        url: &url,
        name: &name,
        intro: &intro,
        icon: icon.as_deref(),
        email: email.as_deref(),
    };
    let id = uuid::Uuid::now_v7().to_string();

    repo::friend::create(&st.conns.pg, &id, LinkStatus::Review.as_str(), &new_link).await?;
    tracing::info!(id = %id, name = %name, "收到友链申请, 状态为待审核");

    Ok(Json(Res::ok(())))
}

#[derive(Debug, Serialize)]
pub struct LikeRes {
    id: String,
    likes: i64,
}

#[instrument(skip_all, fields(article_id = %id))]
pub async fn like_article(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<LikeRes> {
    let likes = repo::article::like(&st.conns.pg, &id)
        .await?
        .ok_or_else(|| AppError::not_found("文章不存在或未发布"))?;

    tracing::info!(article_id = %id, likes, "文章点赞");
    Ok(Json(Res::ok(LikeRes { id, likes })))
}
