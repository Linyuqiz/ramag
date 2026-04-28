//! Tool trait：工具模块统一抽象
//!
//! Ramag 是一个"工具平台"，DB Client 只是众多工具中的一个。
//! 这个 trait 定义了一个 Tool 的元数据（id/名字/图标等），
//! 具体的 UI 渲染由 ui crate 的 ToolView trait 扩展（不在 domain 层）。
//!
//! 这样设计的原因：
//! - Domain 层不能依赖 GPUI（否则测试要拉一堆 GUI 依赖）
//! - 把 UI 渲染独立到 ui crate 的 ToolView trait

use serde::{Deserialize, Serialize};

/// Tool 元数据（不含任何 UI 渲染逻辑）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMeta {
    /// 唯一 id（如 "dbclient"、"jsonfmt"）
    pub id: String,
    /// 用户可见名称
    pub name: String,
    /// 描述
    pub description: String,
    /// 图标名（可选，对应 lucide icon name）
    pub icon: Option<String>,
}

impl ToolMeta {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            icon: None,
        }
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }
}

/// Tool trait：工具模块的最小抽象
///
/// 任何具体工具（DB Client、JSON 格式化、Hash 计算...）实现这个 trait
/// 才能被 ToolRegistry 注册和管理。
pub trait Tool: Send + Sync {
    /// 获取工具元数据
    fn meta(&self) -> &ToolMeta;
}
