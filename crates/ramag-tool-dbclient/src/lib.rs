//! DB Client 工具：MySQL / PostgreSQL / Redis 共用入口

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
use ramag_app::{ConnectionService, MongoService, RedisService};
use ramag_domain::traits::{Tool, ToolMeta};

pub fn create_dbclient_view(
    service: Arc<ConnectionService>,
    redis_service: Arc<RedisService>,
    mongo_service: Arc<MongoService>,
    window: &mut Window,
    cx: &mut App,
) -> AnyView {
    let view = cx.new(|cx| DbClientView::new(service, redis_service, mongo_service, window, cx));
    view.into()
}

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
                "MySQL / PostgreSQL / Redis 客户端，支持元数据浏览、SQL 执行、结果可视化",
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
