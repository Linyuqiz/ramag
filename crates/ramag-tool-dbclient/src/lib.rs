//! Ramag DB Client 工具
//!
//! Stage 0：仅注册 Tool 元数据，视图占位（"Coming Soon"）
//! Stage 2 起：实现完整的连接管理 + 表树
//! Stage 3 起：编辑器 + 多标签
//! Stage 4 起：结果集表格
//! Stage 5/6：历史 + 补全

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod actions;
pub mod sql_completion;
pub mod views;

pub use actions::{
    CopyCellValue, CopySelectedColumn, ExplainQuery, ExportCsv, ExportJson, ExportMarkdown,
    FindInResults, FormatSql, NewQueryTab, RunQuery, RunStatementAtCursor, SaveSqlFile,
    ToggleHistory, ToggleSqlEditor,
};
pub use views::DbClientView;

use std::sync::Arc;

use gpui::{AnyView, App, AppContext as _, Window};
use ramag_app::{ConnectionService, RedisService};
use ramag_domain::traits::{Tool, ToolMeta};

/// 工厂：在 App 上下文创建 DbClientView 并返回 AnyView
pub fn create_dbclient_view(
    service: Arc<ConnectionService>,
    redis_service: Arc<RedisService>,
    window: &mut Window,
    cx: &mut App,
) -> AnyView {
    let view = cx.new(|cx| DbClientView::new(service, redis_service, window, cx));
    view.into()
}

/// DB Client 工具
///
/// 这是 Ramag 的第一个 Tool，也是 v0.1 的核心功能。
pub struct DbClientTool {
    meta: ToolMeta,
}

impl DbClientTool {
    pub const ID: &'static str = "dbclient";

    pub fn new() -> Self {
        Self {
            meta: ToolMeta::new(
                Self::ID,
                "数据库客户端",
                "MySQL GUI 客户端，支持表浏览、SQL 执行、结果可视化",
            )
            .with_icon("database"),
        }
    }
}

impl Default for DbClientTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for DbClientTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_correct() {
        let tool = DbClientTool::new();
        assert_eq!(tool.meta().id, "dbclient");
        assert_eq!(tool.meta().name, "数据库客户端");
        assert!(tool.meta().icon.is_some());
    }
}
