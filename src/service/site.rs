use serde::{Deserialize, Serialize};

use crate::config::AppState;
use crate::model::err::{AppError, AppResult};
use crate::model::handler::site::{AboutSite, HomeSite, HomeSocial, SocialType};
use crate::repo;
use crate::repo::site::{SiteFields, SiteRow};
use crate::repo::social::SocialRow;

const CACHE_KEY: &str = "blrs:site_ctx";
const CACHE_TTL_SECS: u64 = 300;

/// 站点上下文 = 站点信息 + 社交链接。BFF 的每个接口都要它, 所以合并成一次加载 + 一份缓存。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteContext {
    pub site: SiteRow,
    pub social: Vec<SocialRow>,
}

impl SiteContext {
    pub fn home_site(&self) -> HomeSite {
        HomeSite {
            // site_name 允许为空 (首次设密码时会先建一条只有 id 的记录)
            name: self.site.site_name.clone().unwrap_or_default(),
            icon: self.site.site_icon.clone(),
            banner: self.site.banner_image.clone(),
            icp: self.site.icp.clone(),
            copyright: self.site.copyright.clone(),
        }
    }

    pub fn about_site(&self) -> AboutSite {
        AboutSite {
            name: self.site.site_name.clone().unwrap_or_default(),
            icon: self.site.site_icon.clone(),
            banner: self.site.banner_image.clone(),
            icp: self.site.icp.clone(),
            copyright: self.site.copyright.clone(),
            about: self.site.about_content.clone(),
            create_at: self.site.created_at,
        }
    }

    pub fn social(&self) -> Vec<HomeSocial> {
        self.social
            .iter()
            .filter_map(|row| {
                let social_type = match row.social_type.as_str() {
                    "Account" => SocialType::Account,
                    "Link" => SocialType::Link,
                    other => {
                        // 取值只可能来自管理端写入, 走到这里说明有人手改过库。
                        // 记 ERROR 并跳过该行, 不让一个坏行把整个公开站点打挂。
                        tracing::error!(id = %row.id, value = other, "social.type 取值非法, 该行已跳过");
                        return None;
                    }
                };
                Some(HomeSocial {
                    social_type,
                    icon: row.icon.clone(),
                    value: row.value.clone(),
                })
            })
            .collect()
    }
}

/// 读取站点上下文。Valkey 只是缓存 —— 它出任何问题都只记日志并回源数据库,
/// 缓存故障绝不能让公开站点不可用。
pub async fn load(st: &AppState) -> AppResult<SiteContext> {
    let Some(mut conn) = st.conns.valkey() else {
        tracing::debug!("valkey 不可用, 站点上下文直接回源数据库");
        return load_from_db(st).await;
    };

    match redis::cmd("GET")
        .arg(CACHE_KEY)
        .query_async::<Option<String>>(&mut conn)
        .await
    {
        Ok(Some(raw)) => match serde_json::from_str::<SiteContext>(&raw) {
            Ok(ctx) => {
                tracing::debug!(key = CACHE_KEY, "站点上下文命中缓存");
                return Ok(ctx);
            }
            Err(e) => tracing::warn!(error = %e, "站点缓存反序列化失败, 回源数据库"),
        },
        Ok(None) => tracing::debug!(key = CACHE_KEY, "站点上下文缓存未命中"),
        Err(e) => tracing::warn!(error = %e, "valkey 读取失败, 直接回源数据库"),
    }

    let ctx = load_from_db(st).await?;

    if let Ok(raw) = serde_json::to_string(&ctx)
        && let Err(e) = redis::cmd("SET")
            .arg(CACHE_KEY)
            .arg(raw)
            .arg("EX")
            .arg(CACHE_TTL_SECS)
            .query_async::<redis::Value>(&mut conn)
            .await
    {
        tracing::warn!(error = %e, "valkey 写入失败, 本次响应不受影响");
    }

    Ok(ctx)
}

async fn load_from_db(st: &AppState) -> AppResult<SiteContext> {
    let (site, social) = tokio::try_join!(
        repo::site::get(&st.conns.pg),
        repo::social::list(&st.conns.pg)
    )?;

    let Some(site) = site else {
        tracing::error!("site 表没有任何记录, 站点尚未初始化");
        return Err(AppError::internal("站点尚未初始化, 请先创建站点信息"));
    };
    if site.site_name.is_none() {
        // 只有首次设置密码时创建的占位记录, 还没填过站点信息。这里必须报错而不是
        // 返回一个名字为空的站点 —— 静默返回空名字比直接报错更难排查。
        tracing::error!(id = %site.id, "site 记录还没填过站点名称, 视为未初始化");
        return Err(AppError::internal("站点尚未初始化, 请先创建站点信息"));
    }

    Ok(SiteContext { site, social })
}

/// 库里还没有填过内容的站点记录时, 用环境变量注一条进去。
///
/// 站点信息在 Vercel 上本来就是几个环境变量, 但**注入而不是回退**: 读路径只认库里
/// 那一行, 所以公开接口和管理端看到的是同一份数据, 不会出现"线上在生效、管理端却
/// 看不到也改不掉"的幽灵配置; 注入进去之后照样能在 `/admin/site` 改。
///
/// 在迁移之后、开始服务之前调, 每次冷启动跑一次 —— 已经有填过名字的记录就什么都不做
/// (包括不覆盖), 所以改了环境变量不会把管理端的修改冲掉。
pub async fn seed_from_env(st: &AppState) -> AppResult<()> {
    let Some(name) = st.cfg.site.name.as_deref() else {
        tracing::debug!("没有配置 site_name, 跳过站点信息注入");
        return Ok(());
    };

    // 复用管理端建站点那条路径: 它认得"首次设密码时建的占位记录", 会把它补全而不是
    // 撞 409, 一行都没有时才插入。
    let fields = SiteFields {
        name,
        icon: st.cfg.site.icon.as_deref(),
        banner: st.cfg.site.banner.as_deref(),
        // 环境变量里没有 ICP 和关于内容。
        icp: None,
        copyright: st.cfg.site.copyright.as_deref(),
        about: None,
    };

    let id = uuid::Uuid::now_v7().to_string();
    if repo::site::create(&st.conns.pg, &id, &fields).await? {
        tracing::warn!(id = %id, name, "库里没有填过内容的 site 记录, 已用环境变量注入一条");
    } else {
        tracing::debug!("site 记录已存在且填过内容, 环境变量不覆盖它");
    }

    Ok(())
}

/// 站点/社交配置改动后让缓存失效, 这样管理端改完下一个请求就能看到,
/// 不用等 300 秒 TTL 自然过期。
pub async fn invalidate(st: &AppState) {
    let Some(mut conn) = st.conns.valkey() else {
        return;
    };
    match redis::cmd("DEL")
        .arg(CACHE_KEY)
        .query_async::<redis::Value>(&mut conn)
        .await
    {
        Ok(_) => tracing::info!(key = CACHE_KEY, "站点缓存已失效"),
        Err(e) => tracing::warn!(error = %e, "站点缓存失效失败, 最多 300 秒后自然过期"),
    }
}
