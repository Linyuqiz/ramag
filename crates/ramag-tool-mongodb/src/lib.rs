#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! MongoDB 视图，由 dbclient 装载（非独立 Tool）：左 collection 树 + 右多 Tab 查询编辑 + 下结果

pub mod actions;
pub mod views;

pub use actions::{FormatMongoJson, NewMongoQueryTab, RunMongoQuery, ToggleMongoEditor};
pub use views::MongoSessionPanel;
