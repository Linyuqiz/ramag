//! DB Client 快捷键 Action。绑定在 ramag-bin/main.rs 的 `cx.bind_keys`

use gpui::Action;
use schemars::JsonSchema;
use serde::Deserialize;

/// 当前 Query Tab 执行 SQL（默认 cmd-enter）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct RunQuery;

/// 仅执行光标所在那条 SQL（按 `;` 切，默认 cmd-shift-enter）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct RunStatementAtCursor;

/// 新建 Query Tab（默认 cmd-t）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct NewQueryTab;

/// 导出当前结果集为 CSV
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct ExportCsv;

/// 导出当前结果集为 JSON
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct ExportJson;

/// 聚焦结果集过滤栏（默认 cmd-f）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct FindInResults;

/// 格式化当前 SQL（默认 cmd-shift-f）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct FormatSql;

/// EXPLAIN 当前 SQL（默认 cmd-shift-e）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct ExplainQuery;

/// 保存当前编辑器到 .sql 文件（默认 cmd-s）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct SaveSqlFile;

/// 切换查询历史（默认 cmd-shift-h；cmd-h 在 macOS 是隐藏窗口，避开）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct ToggleHistory;

/// 右键菜单：复制选中单元格的完整值
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct CopyCellValue;

/// 工具条「导出」下拉的 Markdown 项：写 .md 文件（按勾选行过滤 / 全部）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct ExportMarkdown;

/// 右键菜单：复制选中单元格所在的列名
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct CopySelectedColumn;

/// 切换 SQL 编辑器显隐（默认 cmd-e；仅控编辑器，工具条 / 结果保留）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct ToggleSqlEditor;
