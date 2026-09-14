use std::env;
use std::sync::Arc;
use std::time::Duration;

use redis::aio::ConnectionManager;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use vercel_blob::client::{VercelBlobApi, VercelBlobClient};

/// 单个连接建立的超时。冷启动时被一个挂住的连接拖死比直接失败更糟。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// migrations/ 下的 SQL 在**编译期**就被嵌进二进制 (展开成 `Cow::Borrowed` 常量),
/// 所以运行时不需要文件系统 —— 这正是 serverless 环境需要的:
/// 函数的工作目录里没有仓库源码, `include_str!` 之外的路子都拿不到这些文件。
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// 迁移用的连接串。
///
/// **必须优先走直连 (NON_POOLING) 地址。** sqlx 的 Postgres 迁移靠 advisory lock
/// 串行化, 而 Supabase 的 `POSTGRES_URL` 是 PgBouncer 事务池 —— 事务池不保证同一
/// 会话, advisory lock 会失效, 并发冷启动时可能两个实例同时跑 DDL。
fn migration_url() -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    match env_url(&["POSTGRES_URL_NON_POOLING"]) {
        Ok(url) => Ok(url),
        Err(_) => {
            tracing::warn!(
                "没有配置 POSTGRES_URL_NON_POOLING, 迁移将走连接池地址; \
                 若该地址经过 PgBouncer (Supabase 默认如此), 迁移可能失败或无法串行化"
            );
            env_url(&["POSTGRES_URL", "POSTGRES_PRISMA_URL"])
        }
    }
}

/// 应用数据库迁移。每次冷启动都会调, 没有待应用的迁移时只是一次查询。
///
/// 失败必须让整个进程起不来: 结构不确定的情况下继续对外服务只会产生更难排查的问题。
pub async fn run_migrations() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let url = migration_url()?;
    let pool = tokio::time::timeout(
        CONNECT_TIMEOUT,
        PgPoolOptions::new().max_connections(1).connect(&url),
    )
    .await
    .map_err(|_| format!("迁移连接超时 ({}s)", CONNECT_TIMEOUT.as_secs()))??;

    let before = applied_versions(&pool).await;

    if let Err(e) = MIGRATOR.run(&pool).await {
        tracing::error!(error = %e, "数据库迁移失败");
        pool.close().await;
        return Err(e.into());
    }

    let after = applied_versions(&pool).await;
    let newly_applied: Vec<i64> = after
        .iter()
        .filter(|version| !before.contains(version))
        .copied()
        .collect();

    if newly_applied.is_empty() {
        tracing::info!(
            known = MIGRATOR.iter().count(),
            "数据库结构已是最新, 无需迁移"
        );
    } else {
        // 结构变更用 WARN 打出来, 免得淹没在常规日志里
        tracing::warn!(?newly_applied, "本次启动了应用了新的数据库迁移");
    }

    pool.close().await;
    Ok(())
}

/// 已成功应用的迁移版本号。
///
/// 仅用于日志。首次运行时 `_sqlx_migrations` 还不存在, 查不到是正常的,
/// 所以这里吞掉错误返回空列表。
async fn applied_versions(pool: &PgPool) -> Vec<i64> {
    sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success = TRUE")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// 应用启动时建立的共享连接 (Postgres + Valkey + Blob)
#[derive(Clone)]
pub struct Connections {
    pub pg: PgPool,
    /// Valkey 只承担缓存和计数, 所以它是**可选**的: 连不上时应用照常启动,
    /// 只是每次都回源数据库。缓存故障绝不能把公开站点拖垮。
    pub valkey: Option<ConnectionManager>,
    pub blob: Arc<VercelBlobClient>,
}

impl Connections {
    pub async fn init() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        // Postgres 是硬依赖, 连不上就别启动了; Valkey 连不上只降级。
        let pg = init_postgres().await;
        let blob = init_blob();

        let mut failures = Vec::new();
        if let Err(e) = &pg {
            failures.push(format!("[postgres] 连接失败: {e}"));
        }
        if let Err(e) = &blob {
            failures.push(format!("[blob] 配置失败: {e}"));
        }
        if !failures.is_empty() {
            for failure in &failures {
                tracing::error!("{failure}");
            }
            return Err(failures.join("; ").into());
        }

        let valkey = match init_valkey().await {
            Ok(conn) => Some(conn),
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "valkey 连接失败, 将以无缓存模式运行 (缓存/计数/登录封禁暂时失效)"
                );
                None
            }
        };

        tracing::info!(
            cache_enabled = valkey.is_some(),
            "数据库连接已建立 (postgres / valkey / blob)"
        );
        Ok(Self {
            pg: pg?,
            valkey,
            blob: blob?,
        })
    }

    /// 取一个 Valkey 连接。返回 `None` 表示启动时就没连上 —— 所有调用方都必须能
    /// 接受"没有缓存"这个情况, 记一条 WARN 然后走数据库。
    pub fn valkey(&self) -> Option<ConnectionManager> {
        self.valkey.clone()
    }

    /// 逐个测试连通性并打印结果。失败只记日志, 不阻止启动。
    pub async fn check(&self) {
        match sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.pg)
            .await
        {
            Ok(v) => tracing::info!(value = v, "[postgres] 连通正常"),
            Err(e) => tracing::error!(error = %e, "[postgres] 连通检查失败"),
        }

        match self.valkey() {
            None => tracing::error!("[valkey] 启动时未建立连接, 缓存相关功能已降级"),
            Some(mut conn) => match redis::cmd("PING").query_async::<String>(&mut conn).await {
                Ok(pong) => tracing::info!(response = %pong, "[valkey] 连通正常"),
                Err(e) => tracing::error!(error = %e, "[valkey] 连通检查失败"),
            },
        }

        match self.blob.list(Default::default()).await {
            Ok(list) => tracing::info!(blobs = list.blobs.len(), "[blob] 连通正常"),
            Err(e) => tracing::error!(error = %e, "[blob] 连通检查失败"),
        }
    }
}

async fn init_postgres() -> Result<PgPool, Box<dyn std::error::Error + Send + Sync>> {
    // Supabase/Vercel Postgres, 优先使用连接池地址
    let url = env_url(&["POSTGRES_URL", "POSTGRES_PRISMA_URL", "POSTGRES_URL_NON_POOLING"])?;
    let pool = tokio::time::timeout(
        CONNECT_TIMEOUT,
        PgPoolOptions::new().max_connections(1).connect(&url),
    )
    .await
    .map_err(|_| format!("postgres 连接超时 ({}s)", CONNECT_TIMEOUT.as_secs()))??;
    Ok(pool)
}

async fn init_valkey() -> Result<ConnectionManager, Box<dyn std::error::Error + Send + Sync>> {
    // Aiven Valkey
    let url = env_url(&["aiven_valkey_service_uri", "AIVEN_VALKEY_SERVICE_URI"])?;
    // redis-rs 只识别 redis(s):// 协议头, 这里兼容 valkey(s)://
    let url = url
        .replacen("valkeys://", "rediss://", 1)
        .replacen("valkey://", "redis://", 1);
    let client = redis::Client::open(url)?;
    let conn = tokio::time::timeout(CONNECT_TIMEOUT, client.get_connection_manager())
        .await
        .map_err(|_| format!("valkey 连接超时 ({}s)", CONNECT_TIMEOUT.as_secs()))??;
    Ok(conn)
}

fn init_blob() -> Result<Arc<VercelBlobClient>, Box<dyn std::error::Error + Send + Sync>> {
    env_url(&["BLOB_READ_WRITE_TOKEN"])?;
    // VercelBlobClient::new() 在请求时才读取 BLOB_READ_WRITE_TOKEN, 这里提前校验
    Ok(Arc::new(VercelBlobClient::new()))
}

fn env_url(keys: &[&str]) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    for key in keys {
        if let Ok(value) = env::var(key)
            && !value.is_empty()
        {
            return Ok(value);
        }
    }
    Err(format!("缺少环境变量: {}", keys.join(" / ")).into())
}
