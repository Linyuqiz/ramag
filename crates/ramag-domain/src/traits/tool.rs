//! Tool trait：工具元数据。UI 渲染由 ramag-ui 的 ToolView 扩展，不放 domain

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMeta {
    /// 唯一 id，如 "dbclient"
    pub id: String,
    pub name: String,
    pub description: String,
    /// lucide icon name
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

/// 工具最小抽象，实现该 trait 才能被 ToolRegistry 注册
pub trait Tool: Send + Sync {
    fn meta(&self) -> &ToolMeta;
}
