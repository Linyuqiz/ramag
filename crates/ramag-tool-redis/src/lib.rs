// 测试代码大量使用 unwrap/expect/panic（断言失败即阻断），是 Rust 测试的常态
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! Ramag Redis 视图组件库
//!
//! Stage 14：数据层（Domain + Infra Driver）已完成
//! Stage 15：UI 视图 — 由 dbclient 工具装载（不再是独立 Tool）
//!   - [`RedisSessionPanel`]：连接打开后的会话面板（DB 切换 + Key 树 + 详情）
//!   - [`views::key_tree::KeyTreePanel`]：Key 列表
//!   - [`views::key_detail::KeyDetailPanel`]：值详情
//!
//! 后续 Stage：CLI 命令面板 / Pub/Sub / Streams / 监控

pub mod actions;
pub mod views;

pub use actions::{DeleteCurrentKey, RefreshKeyTree};
pub use views::RedisSessionPanel;
