//! 领域实体模块
//!
//! 核心业务概念的数据结构定义，纯 Rust 类型，可序列化。

pub mod connection;
pub mod history;
pub mod query;
pub mod schema;

pub use connection::{ConnectionColor, ConnectionConfig, ConnectionId, DriverKind};
pub use history::{QueryRecord, QueryRecordId, QueryStatus};
pub use query::{Query, QueryResult, Row, Value, Warning};
pub use schema::{Column, ColumnKind, ColumnType, ForeignKey, Index, Schema, Table};
