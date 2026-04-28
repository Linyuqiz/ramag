//! Ramag 领域层
//!
//! 这个 crate 是整个项目的核心，定义了：
//! - 核心实体（entities）：Connection、Query、QueryResult、Schema、Table 等
//! - 抽象 trait（traits）：Driver、Storage、Tool
//!
//! 设计原则：
//! 1. 不依赖任何 UI 框架（GPUI）或具体技术实现（sqlx、redb）
//! 2. 仅依赖纯工具类 crate（serde、thiserror、async-trait）
//! 3. 任何模块都可以引用 domain，但 domain 不引用任何业务模块
//! 4. 所有数据库特化逻辑通过 trait 抽象，infra 层实现

pub mod entities;
pub mod error;
pub mod traits;

// 重导出，让上层使用更简洁
pub use error::{DomainError, Result};
pub use traits::{Driver, KvDriver, Storage, Tool, ToolMeta};
