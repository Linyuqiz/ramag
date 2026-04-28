//! DB Client 工具的快捷键 Action 定义
//!
//! 这些 Action 由 ramag-bin/main.rs 在启动时通过 `cx.bind_keys` 绑定，
//! 视图通过 `cx.on_action` 或 `cx.listener` 注册处理逻辑。

use gpui::Action;
use schemars::JsonSchema;
use serde::Deserialize;

/// 在当前 Query Tab 执行 SQL（默认 ⌘↵ / Cmd+Enter）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct RunQuery;

/// 仅执行光标所在的那条 SQL（多语句以 `;` 分隔，默认 ⌘⇧↵ / Cmd+Shift+Enter）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct RunStatementAtCursor;

/// 新建一个 Query Tab（默认 ⌘T / Cmd+T）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct NewQueryTab;

/// 关闭当前 Query Tab（默认 ⌘W / Cmd+W）
///
/// 多 tab 时 QueryPanel 会消费事件关闭当前 tab；
/// 仅剩 1 个 tab 时事件冒泡到 main.rs 的全局 fallback 关闭窗口（VSCode 风格）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct CloseQueryTab;

/// 导出当前结果集为 CSV
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct ExportCsv;

/// 导出当前结果集为 JSON
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct ExportJson;

/// 聚焦结果集过滤栏（默认 ⌘F / Cmd+F）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct FindInResults;

/// 格式化当前 SQL（默认 ⌘⇧F / Cmd+Shift+F）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct FormatSql;

/// EXPLAIN 当前 SQL（默认 ⌘⇧E / Cmd+Shift+E）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct ExplainQuery;

/// 保存当前编辑器内容为 .sql 文件（默认 ⌘S / Cmd+S）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct SaveSqlFile;

/// 切换查询历史面板（默认 ⌘⇧H / Cmd+Shift+H；macOS 的 ⌘H 是隐藏窗口，避开冲突）
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

/// 切换 SQL 编辑器显隐（默认 ⌘E；只控编辑器那一块，工具条/结果保留）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_dbclient)]
pub struct ToggleSqlEditor;
