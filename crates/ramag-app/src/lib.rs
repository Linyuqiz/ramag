//! Ramag 应用层
//!
//! 这一层负责：
//! - **Use Cases**：业务用例编排（核心是 ConnectionService，聚合连接 / 元数据 / 执行 / 历史）
//! - **ToolRegistry**：管理已注册的 Tool 实例
//!
//! 它依赖 Domain trait（Driver、Storage、Tool），但不关心具体实现。
//! UI 层调用 Use Case 来完成业务操作。

pub mod tool_registry;
pub mod usecases;

pub use tool_registry::ToolRegistry;
pub use usecases::{ConnectionService, RedisService};
