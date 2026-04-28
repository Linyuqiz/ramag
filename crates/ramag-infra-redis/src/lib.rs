// 测试代码大量使用 unwrap/expect/panic（断言失败即阻断），是 Rust 测试的常态
// cfg_attr(test, ...) 只在 test 配置下放行，不影响生产代码的严格审计
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! Ramag Redis 驱动实现
//!
//! 实现 [`ramag_domain::traits::KvDriver`]，封装 redis-rs 的
//! `aio::ConnectionManager`（自动重连 + 多路复用）。
//!
//! # 设计要点
//!
//! - **连接缓存**：按 `(ConnectionId, db)` 维度缓存 ConnectionManager
//!   （Redis SELECT 是连接级状态，不能跨 db 共享）
//! - **双 runtime 桥接**：通过 [`runtime::run_in_tokio`] 把 redis 调用派发到
//!   独立 tokio runtime（与 mysql 共存但各持一份，互不干扰）
//! - **类型映射**：覆盖 RESP2/3 全部 14 种类型 → Domain `RedisValue`
//!   （[`value::decode_value`] 等）
//! - **错误转换**：Redis 错误前缀（NOAUTH/WRONGTYPE/OOM/READONLY 等）
//!   → 中文友好提示（[`errors::map_redis_error`]）
//!
//! # 用法
//!
//! ```no_run
//! use std::sync::Arc;
//! use ramag_domain::traits::KvDriver;
//! use ramag_domain::entities::ConnectionConfig;
//! use ramag_infra_redis::RedisDriver;
//!
//! # async fn demo() -> ramag_domain::error::Result<()> {
//! let driver: Arc<dyn KvDriver> = Arc::new(RedisDriver::new());
//! let config = ConnectionConfig::new_redis("local", "127.0.0.1", 6379);
//! driver.test_connection(&config).await?;
//! # Ok(()) }
//! ```

pub mod driver;
pub mod errors;
pub mod pool;
pub mod runtime;
pub mod value;

pub use driver::RedisDriver;
