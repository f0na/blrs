//! 数据访问层。
//!
//! 这是**唯一**出现 `sqlx` 的地方。上层 (service/handler) 只面对这里的结构体和方法,
//! 不直接接触 `PgPool`。
//!
//! 全部使用运行时 `query_as` + `FromRow` 而不是 `query!` 宏: 宏需要编译期
//! `DATABASE_URL`, 会直接破坏 Vercel 的构建。
//!
//! # 每条查询都必须带 `.persistent(false)`
//!
//! 应用连的是 Supabase 的 PgBouncer **事务池**(6543 端口)。sqlx 默认使用**具名**
//! 预处理语句 (`sqlx_s_1`, `sqlx_s_2`, ...), 而具名语句是**会话级**的:
//! 事务池在事务之间会把同一个客户端连接丢到不同的 Postgres 后端上, 于是两个 sqlx
//! 连接各自创建的 `sqlx_s_1` 可能落到同一个后端, 报:
//!
//! ```text
//! prepared statement "sqlx_s_1" already exists
//! ```
//!
//! 注意 `statement_cache_capacity(0)` **解决不了**这个问题 —— 看 sqlx 源码,
//! 语句名是否带 `sqlx_s_` 前缀只取决于 `persistent`, 缓存容量只决定要不要记住它。
//! 只有 `.persistent(false)` 才会让 sqlx 发**匿名**语句, 从根上避开命名冲突。
//!
//! 代价是每条查询都要重新 Parse 一次。对这个体量的站点可以忽略。
//! `tests::every_query_disables_persistent` 会守住这条规则, 新加查询漏了就编译期过不了测试。

pub mod article;
pub mod friend;
pub mod mail;
pub mod site;
pub mod social;

#[cfg(test)]
mod tests {
    use crate::repo;

    /// 防止新加的查询漏掉 `.persistent(false)` —— 漏了的话不会编译报错,
    /// 只会在线上随机冒出 `prepared statement "sqlx_s_1" already exists`。
    ///
    /// 做法: 数一数每个 repo 文件里 `sqlx::query*` 构造的个数, 和
    /// `.persistent(false)` 的个数, 两者必须相等。
    #[test]
    fn every_query_disables_persistent() {
        let files: [(&str, &str); 5] = [
            ("site.rs", include_str!("site.rs")),
            ("social.rs", include_str!("social.rs")),
            ("article.rs", include_str!("article.rs")),
            ("friend.rs", include_str!("friend.rs")),
            ("mail.rs", include_str!("mail.rs")),
        ];

        for (name, source) in files {
            let constructors = source.matches("sqlx::query").count();
            let disabled = source.matches(".persistent(false)").count();
            assert_eq!(
                constructors, disabled,
                "{name}: 有 {constructors} 处 sqlx::query* 构造, 但只有 {disabled} 处带了 \
                 .persistent(false) —— 漏掉的那些在 PgBouncer 事务池上会报 prepared statement already exists"
            );
        }
    }

    /// 这个测试本身不该被上面的扫描误伤 (mod.rs 不在扫描列表里), 顺带确认模块可见。
    #[test]
    fn repo_modules_are_reachable() {
        let _ = std::any::type_name::<repo::site::SiteRow>();
    }
}
