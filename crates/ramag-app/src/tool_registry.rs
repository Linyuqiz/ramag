//! ToolRegistry：工具注册中心
//!
//! 在程序启动时（main.rs）把所有 Tool 实例注册进来，
//! UI 层从 Registry 拿到列表渲染左侧导航。

use std::sync::Arc;

use parking_lot::RwLock;
use ramag_domain::Tool;

/// 工具注册中心
///
/// 内部用 `RwLock<Vec<Arc<dyn Tool>>>`，多线程安全（GUI 线程 + 后台任务都可读）
#[derive(Default)]
pub struct ToolRegistry {
    tools: RwLock<Vec<Arc<dyn Tool>>>,
}

impl ToolRegistry {
    /// 创建一个空的 Registry
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一个 Tool
    pub fn register(&self, tool: Arc<dyn Tool>) {
        let mut tools = self.tools.write();
        // 防重复注册（基于 id 比较）
        if tools.iter().any(|t| t.meta().id == tool.meta().id) {
            tracing::warn!(tool_id = %tool.meta().id, "duplicate tool registration ignored");
            return;
        }
        tracing::info!(tool_id = %tool.meta().id, name = %tool.meta().name, "tool registered");
        tools.push(tool);
    }

    /// 列出所有 Tool（按注册顺序）
    pub fn list(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.read().clone()
    }

    /// 根据 id 查找 Tool
    pub fn find(&self, id: &str) -> Option<Arc<dyn Tool>> {
        self.tools.read().iter().find(|t| t.meta().id == id).cloned()
    }

    /// 当前已注册数量
    pub fn count(&self) -> usize {
        self.tools.read().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ramag_domain::ToolMeta;

    struct DummyTool {
        meta: ToolMeta,
    }

    impl Tool for DummyTool {
        fn meta(&self) -> &ToolMeta {
            &self.meta
        }
    }

    #[test]
    fn register_and_list() {
        let reg = ToolRegistry::new();
        let t1 = Arc::new(DummyTool {
            meta: ToolMeta::new("a", "ToolA", "first"),
        });
        let t2 = Arc::new(DummyTool {
            meta: ToolMeta::new("b", "ToolB", "second"),
        });
        reg.register(t1);
        reg.register(t2);
        assert_eq!(reg.count(), 2);
        assert!(reg.find("a").is_some());
        assert!(reg.find("missing").is_none());
    }

    #[test]
    fn duplicate_registration_ignored() {
        let reg = ToolRegistry::new();
        let t1 = Arc::new(DummyTool {
            meta: ToolMeta::new("dup", "Tool1", ""),
        });
        let t2 = Arc::new(DummyTool {
            meta: ToolMeta::new("dup", "Tool2", ""),
        });
        reg.register(t1);
        reg.register(t2);
        assert_eq!(reg.count(), 1);
    }
}
