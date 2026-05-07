//! 领域 trait 集合。infra 层实现，app 层依赖

pub mod driver;
pub mod git_driver;
pub mod kv_driver;
pub mod storage;
pub mod tool;

pub use driver::{CancelHandle, Driver};
pub use git_driver::GitDriver;
pub use kv_driver::KvDriver;
pub use storage::Storage;
pub use tool::{Tool, ToolMeta};
