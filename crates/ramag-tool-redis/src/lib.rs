#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! Redis 视图，由 dbclient 装载（非独立 Tool）：DB 切换 + Key 树 + 详情

pub mod views;

pub use views::RedisSessionPanel;
