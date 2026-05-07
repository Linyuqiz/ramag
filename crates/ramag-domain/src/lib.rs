//! 领域层：实体 + trait 抽象。不依赖 UI / sqlx / redb / redis。

pub mod entities;
pub mod error;
pub mod traits;

pub use error::{DomainError, Result};
pub use traits::{Driver, KvDriver, Storage, Tool, ToolMeta};
