use std::time::Duration;

use crate::config::AppState;
use crate::model::LinkStatus;
use crate::model::err::{AppError, AppResult};
use crate::repo;
use crate::service::mail;

/// 发信的超时上限。
const MAIL_TIMEOUT: Duration = Duration::from_secs(5);

/// 拒绝文案的长度上限。
pub const MAX_REFUSE_MESSAGE_LEN: usize = 1000;

#[derive(Debug, Clone, Copy)]
pub struct Decision {
    /// 通知邮件是否发出。
    pub notified: bool,
}

/// 审核通过。
///
/// **先提交事务再发信。** 这是"发信失败不回滚审核"的实现机制: 走到 SMTP 时事务
/// 已经关闭, 邮件发不出去只体现为 `notified = false`, 接口照样 200。
pub async fn approve(st: &AppState, id: &str) -> AppResult<Decision> {
    let mut tx = st.conns.pg.begin().await?;

    let Some(row) = repo::friend::get_for_update(&mut tx, id).await? else {
        return Err(AppError::not_found("友链不存在"));
    };

    if row.feedback_status == LinkStatus::Pass.as_str() {
        tracing::info!(id, "友链已经是通过状态, 幂等跳过, 不重复发信");
        tx.rollback().await?;
        return Ok(Decision { notified: false });
    }

    repo::friend::set_status(&mut tx, id, LinkStatus::Pass.as_str()).await?;
    tx.commit().await?;
    tracing::info!(id, "友链审核已通过");

    let notified = match row.email.as_deref() {
        None => {
            tracing::info!(id, "该友链没有留邮箱, 无需通知");
            false
        }
        Some(to) => match send(st, to, &row.site_name, &row.site_url, None).await {
            Ok(()) => {
                tracing::info!(id, "友链通过通知已发送");
                true
            }
            Err(e) => {
                // 审核结果已经落库了, 这里只记错误, 不改接口结果。
                tracing::error!(id, error = %e, "友链通过通知发送失败");
                false
            }
        },
    };

    Ok(Decision { notified })
}

/// 拒绝。带上管理员编辑的说明, 发信之后把记录删掉。
///
/// 这里的顺序和 `approve` **相反**, 是刻意的: 拒绝时记录本身就是要被删掉的东西,
/// 如果先删再发信、而信又没发出去, 管理员手上就再没有任何东西可以重试了。
/// 所以先发信: 发不出去就直接返回错误, 记录原样保留, 管理员可以改完配置再点一次。
/// 只有在对方压根没留邮箱这种没法通知的情况下, 才直接删掉。
pub async fn refuse(st: &AppState, id: &str, message: &str) -> AppResult<Decision> {
    let message = message.trim();
    if message.is_empty() {
        return Err(AppError::bad_request("拒绝说明不能为空"));
    }
    if message.chars().count() > MAX_REFUSE_MESSAGE_LEN {
        return Err(AppError::bad_request(format!(
            "拒绝说明不能超过 {MAX_REFUSE_MESSAGE_LEN} 字"
        )));
    }

    // 只读一次, 不开事务: 发信要等 SMTP, 不能占着那条唯一的数据库连接。
    let row = repo::friend::get_mail_row(&st.conns.pg, id)
        .await?
        .ok_or_else(|| AppError::not_found("友链不存在"))?;

    let Some(to) = row.email.as_deref() else {
        tracing::info!(id, "该友链没有留邮箱, 直接删除记录");
        return delete_row(st, id, false).await;
    };

    send(st, to, &row.site_name, &row.site_url, Some(message))
        .await
        .inspect_err(|e| tracing::error!(id, error = %e, "友链拒绝通知发送失败, 记录保留待重试"))?;
    tracing::info!(id, "友链拒绝通知已发送");

    delete_row(st, id, true).await
}

async fn delete_row(st: &AppState, id: &str, notified: bool) -> AppResult<Decision> {
    if !repo::friend::delete(&st.conns.pg, id).await? {
        return Err(AppError::not_found("友链不存在"));
    }
    tracing::info!(id, "友链记录已删除");
    Ok(Decision { notified })
}

/// 发一封决定通知。**邮箱地址只在这个函数里出现** —— 日志和响应里都只有 id。
async fn send(
    st: &AppState,
    to: &str,
    site_name: &str,
    site_url: &str,
    refuse_message: Option<&str>,
) -> AppResult<()> {
    let sending = async {
        match refuse_message {
            Some(message) => mail::send_friend_refused(st, to, site_name, message).await,
            None => mail::send_friend_approved(st, to, site_name, site_url).await,
        }
    };
    match tokio::time::timeout(MAIL_TIMEOUT, sending).await {
        Ok(result) => result,
        Err(_) => Err(AppError::upstream(format!(
            "发信超时 ({}s)",
            MAIL_TIMEOUT.as_secs()
        ))),
    }
}
