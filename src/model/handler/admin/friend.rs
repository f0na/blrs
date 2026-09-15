use serde::{Deserialize, Serialize};

use crate::model::LinkStatus;

/// 管理端可见的友链。
///
/// **物理上没有 email 字段**: 通知邮箱任何读接口都不得返回, 所以这里连字段都不存在。
/// 即使将来有人把 SQL 改成 `SELECT *`, 也没地方放这个值。
#[derive(Debug, Clone, Serialize)]
pub struct FriendLinkAdmin {
    pub id: String,
    pub url: String,
    pub name: String,
    pub intro: String,
    pub icon: Option<String>,
    pub feedback_status: LinkStatus,
    /// Unix 秒, 原样返回。
    pub create_at: i64,
}

/// 管理端新增友链 (访客申请走公开接口, 形状相同)。
#[derive(Debug, Clone, Deserialize)]
pub struct FriendLinkInput {
    pub url: String,
    pub name: String,
    pub intro: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

/// 拒绝友链时的文案, 由管理员自己编辑。
#[derive(Debug, Clone, Deserialize)]
pub struct RefuseInput {
    pub message: String,
}

/// 审核通过 / 拒绝的结果。
///
/// `notified` 表示通知邮件是否发出。发信失败**不是**错误: 业务操作已经完成,
/// 所以这种情况依然返回 200, 只是 `notified = false`。
#[derive(Debug, Clone, Serialize)]
pub struct DecideRes {
    pub id: String,
    pub notified: bool,
}
