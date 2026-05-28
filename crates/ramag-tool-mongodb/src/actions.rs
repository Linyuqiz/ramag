//! MongoDB 视图专属快捷键 Action。绑定在 ramag-bin/main.rs 的 `cx.bind_keys`

use gpui::Action;
use schemars::JsonSchema;
use serde::Deserialize;

/// 当前 Query Tab 执行 Mongo 命令（默认 cmd-enter，与 SQL 共享键位但走 ramag_mongodb 命名空间）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_mongodb)]
pub struct RunMongoQuery;

/// 新建 Query Tab（默认 cmd-t）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_mongodb)]
pub struct NewMongoQueryTab;

/// 格式化当前编辑器 JSON（默认 cmd-shift-f）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_mongodb)]
pub struct FormatMongoJson;

/// 切换 JSON 命令编辑器显隐（默认 cmd-e；与 dbclient ToggleSqlEditor 同样的交互习惯）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_mongodb)]
pub struct ToggleMongoEditor;
