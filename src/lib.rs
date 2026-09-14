// 业务代码全在这里, `api/` 下只留 `axum.rs` 一个入口。
//
// Vercel 的零配置会把 `api/` 下**每一个** `.rs` 都当成一个独立的 serverless
// function 去构建 (CLI 里写死的 `api/**/*.rs` -> `@vercel/rust`), 而每个 function
// 都要求 `Cargo.toml` 里有一条 path 指向它的 `[[bin]]`。所以除了真正的入口,
// 其他 `.rs` 一旦放进 `api/` 就会让整个构建失败。

pub mod config;
pub mod db;
pub mod handler;
pub mod middleware;
pub mod model;
pub mod repo;
pub mod service;
pub mod util;
