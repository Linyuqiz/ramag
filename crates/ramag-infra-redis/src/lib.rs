#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! Redis driver。impl KvDriver。封装 redis-rs `aio::ConnectionManager`（自动重连 + 多路复用）。
//! 连接缓存按 `(ConnectionId, db)`（SELECT 是连接级状态，不能跨 db 共享）；走独立 tokio runtime

pub mod command;
pub mod driver;
pub mod errors;
pub mod pool;
pub mod runtime;
pub mod value;

pub use driver::RedisDriver;
