use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::config::AppState;
use crate::model::err::{AppError, AppResult};
use crate::repo;
use crate::util::sanitize_header_value;

/// 实际用于发信的一套配置。
#[derive(Debug, Clone)]
pub struct ResolvedMail {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from: String,
}

/// 解析邮件配置: 优先用库里**唯一启用**的那条 `mail_config`, 没有才回退到环境变量。
///
/// 环境变量在启动时就已经校验过必填, 所以回退路径一定拿得到值。
/// 每次都重新解析 (不做缓存) —— 管理端改了配置必须立刻生效, 而发信量本来极小。
pub async fn resolve(st: &AppState) -> AppResult<ResolvedMail> {
    if let Some(row) = repo::mail::get_enabled(&st.conns.pg).await? {
        let port = u16::try_from(row.port).map_err(|_| {
            tracing::error!(id = %row.id, port = row.port, "mail_config 里的端口非法");
            AppError::internal("mail_config 端口非法")
        })?;
        tracing::debug!(id = %row.id, "使用数据库中启用的邮件配置");
        return Ok(ResolvedMail {
            host: row.host,
            port,
            username: row.username,
            password: row.password,
            from: row.from_addr,
        });
    }

    tracing::debug!("没有启用的 mail_config, 回退到环境变量");
    let env = &st.cfg.mail;
    Ok(ResolvedMail {
        host: env.host.clone(),
        port: env.port,
        username: env.username.clone(),
        password: env.password.clone(),
        from: env.from.clone(),
    })
}

/// 发一封纯文本邮件。
///
/// transport 按次构建: 配置可能随时被管理端改掉, 缓存的 transport 会和配置脱节。
/// 管理端流量极小, 这点开销可以接受。
pub async fn send(st: &AppState, to: &str, subject: &str, body: String) -> AppResult<()> {
    let cfg = resolve(st).await?;

    let from: Mailbox = cfg
        .from
        .parse()
        .map_err(|e| AppError::internal(format!("mail_from 不是合法地址: {e}")))?;
    // 不要把解析错误原文放进 msg: lettre 的错误里会带上地址本身, 而这个错误
    // 最终会被记进日志。邮箱地址只允许停留在发信函数内部。
    let to: Mailbox = to
        .parse()
        .map_err(|_| AppError::internal("库里的收件地址不是合法邮箱, 无法投递"))?;

    let email = Message::builder()
        .from(from)
        .to(to)
        // 主题里的 CRLF 必须先剥掉, 否则可以注入额外的邮件头
        .subject(sanitize_header_value(subject))
        .body(body)
        .map_err(|e| AppError::internal(format!("构造邮件失败: {e}")))?;

    let mut builder = AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host)?
        .port(cfg.port)
        .credentials(Credentials::new(cfg.username.clone(), cfg.password.clone()));

    // 465 是隐式 TLS (连上就开始握手), 其余端口走 STARTTLS。
    // relay() 默认就是 STARTTLS, 所以只需要覆盖 465 这一种。
    if cfg.port == 465 {
        let params = TlsParameters::new(cfg.host.clone())
            .map_err(|e| AppError::internal(format!("构造 SMTP TLS 参数失败: {e}")))?;
        builder = builder.tls(Tls::Wrapper(params));
    }

    builder.build().send(email).await?;
    Ok(())
}

pub async fn send_friend_approved(
    st: &AppState,
    to: &str,
    site_name: &str,
    site_url: &str,
) -> AppResult<()> {
    let subject = format!("友链申请已通过 - {site_name}");
    let body = format!(
        "你好,\n\n\
         你提交的友链申请已经通过审核, 现在可以在站点上看到了。\n\n\
         站点名称: {site_name}\n\
         站点地址: {site_url}\n\n\
         感谢你的支持。\n"
    );
    send(st, to, &subject, body).await
}

/// 拒绝文案由管理员填写, 这里只做长度上限校验后原样放进正文。
pub async fn send_friend_refused(
    st: &AppState,
    to: &str,
    site_name: &str,
    message: &str,
) -> AppResult<()> {
    let subject = format!("友链申请未通过 - {site_name}");
    let body = format!(
        "你好,\n\n\
         你提交的友链申请未能通过审核。\n\n\
         说明:\n{message}\n\n\
         感谢你的关注。\n"
    );
    send(st, to, &subject, body).await
}
