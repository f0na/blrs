use serde::{Deserialize, Serialize};

use crate::model::handler::site::SocialType;

#[derive(Debug, Clone, Deserialize)]
pub struct SocialInput {
    pub social_type: SocialType,
    #[serde(default)]
    pub icon: Option<String>,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SocialAdmin {
    pub id: String,
    pub social_type: SocialType,
    pub icon: Option<String>,
    pub value: String,
    /// Unix 秒, 原样返回。
    pub create_at: i64,
}
