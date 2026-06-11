// 测试场景放开 unwrap/expect/panic（断言失败即阻断），不影响生产代码审计
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! 应用层：Use Cases + ToolRegistry。依赖 domain trait，不持具体实现

pub mod tool_registry;
pub mod usecases;

pub use tool_registry::ToolRegistry;
pub use usecases::{ClipboardService, ConnectionService, MongoService, RedisService};
