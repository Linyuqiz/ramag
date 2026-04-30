//! Ramag SQL 共享层
//!
//! 关系型 DB driver（MySQL / PostgreSQL / 未来 SQLite 等）的唯一抽象层。
//! 每个 driver crate 只 impl 一个 [`SqlBackend`] trait，再用 [`impl_driver_for!`]
//! 宏一行获得 Domain 层的 `Driver` trait 实现。
//!
//! # 设计
//!
//! - [`runtime`]：tokio↔smol 桥接（GPUI 是 smol，sqlx 强依赖 tokio）
//! - [`sql`]：SQL 文本工具（多语句切分、LIMIT 注入），含 dollar-quoted 选项
//! - [`errors`]：sqlx::Error 通用大类映射；DB 错误码表留给 driver crate
//! - [`backend`]：[`SqlBackend`] trait + 泛型模板函数（test/execute/metadata 委托）
//! - [`pool`]：泛型 PoolCache&lt;Db&gt;
//! - [`macros`]：[`impl_driver_for!`] 宏

pub mod backend;
pub mod errors;
pub mod macros;
pub mod pool;
pub mod runtime;
pub mod sql;

pub use backend::{
    SqlBackend, cancel_query_impl, execute_impl, list_columns_impl, list_foreign_keys_impl,
    list_indexes_impl, list_schemas_impl, list_tables_impl, server_version_impl,
    test_connection_impl,
};
pub use pool::PoolCache;
pub use runtime::run_in_tokio;
