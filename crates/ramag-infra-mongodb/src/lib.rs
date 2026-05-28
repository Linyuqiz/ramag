#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! MongoDB driver。impl DocDriver。封装 mongodb 官方 Client（自带连接池 + 自动重连）。
//! 连接缓存按 `ConnectionId`（mongo 的 db 切换是命令级而非连接级，与 SQL 一致）；走独立 tokio runtime

pub mod driver;
pub mod errors;
pub mod metadata;
pub mod pool;
pub mod query;
pub mod runtime;
pub mod types;

pub use driver::MongoDriver;
