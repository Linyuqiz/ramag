//! 领域 trait 集合
//!
//! 所有抽象接口集中在这里。Infra 层实现这些 trait，App 层依赖这些 trait。

pub mod driver;
pub mod storage;
pub mod tool;

pub use driver::{CancelHandle, Driver};
pub use storage::Storage;
pub use tool::{Tool, ToolMeta};
