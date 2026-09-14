//! 管理端 handler, 按资源类型划分。
//!
//! 访问量极小, 所以没有做任何按动作细分的设计: 一个资源一组路由, 方法表达动作。

pub mod article;
pub mod auth;
pub mod blob;
pub mod friend;
pub mod mail;
pub mod search;
pub mod site;
pub mod social;
