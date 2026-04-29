// 测试代码大量使用 unwrap/expect/panic（断言失败即阻断），是 Rust 测试的常态
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! Ramag Redis 视图组件库
//!
//! 由 dbclient 工具装载（不再是独立 Tool）。当前仅提供：
//! - [`RedisSessionPanel`]：连接打开后的会话面板（DB 切换 + Key 树 + 详情）
//! - [`views::key_tree::KeyTreePanel`] / [`views::key_detail::KeyDetailPanel`]
//!
//! CLI / Pub/Sub 入口已移除；后续如要重做请新建独立模块

pub mod views;

pub use views::RedisSessionPanel;
