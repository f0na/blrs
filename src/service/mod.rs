//! 业务层。
//!
//! handler 只负责解析/校验/塑形 HTTP; 事务、发信、缓存失效、令牌签发这类编排都在这里。
//! 这里不构造 HTTP 类型, 只返回领域结构体和 [`AppError`](crate::model::err::AppError)。

pub mod article;
pub mod auth;
pub mod ban;
pub mod blob;
pub mod friend;
pub mod mail;
pub mod search;
pub mod site;
