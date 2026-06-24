#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! Redis 视图，由 dbclient 装载（非独立 Tool）：DB 切换 + Key 树 + 详情

pub mod actions;
pub mod views;

pub use actions::ToggleRedisConsole;
pub use views::RedisSessionPanel;
