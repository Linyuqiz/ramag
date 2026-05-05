//! 结果集面板
//!
//! 三种渲染状态：Empty / Running / Error / Ok。
//!
//! 模块拆分：
//! - 本文件：state + new + 公共 setter/getter（`ResultState` / `SortDir` enum）
//! - [`ops`]：DML 操作（INSERT/UPDATE/DELETE，行内编辑触发）
//! - [`render`]：`impl Render` + 警告 banner + 复制操作
//! - [`export`]：CSV/JSON/Markdown 导出
//! - [`helpers`]：自由函数 + `PendingInsert` struct

mod export;
mod helpers;
mod ops;
mod render;

pub use export::ExportFormat;

use std::collections::BTreeSet;
use std::sync::Arc;

use parking_lot::RwLock;

use gpui::{
    AppContext as _, Context, Entity, Point, ScrollHandle, ScrollStrategy, UniformListScrollHandle,
    Window, px,
};
use gpui_component::input::InputState;
use gpui_component::notification::Notification;
use ramag_app::ConnectionService;
use ramag_domain::entities::{Column, ConnectionConfig, QueryResult, Value};

use crate::sql_completion::SchemaCache;
use helpers::{PendingInsert, extract_first_table_ref, find_pk_idx, parse_value_for_kind};

/// UI 表格最多渲染行数（超出截断 + 状态栏提示"已截断"）
pub(super) const MAX_ROWS_DISPLAY: usize = 10_000;

/// 结果集状态
#[derive(Debug, Clone, Default)]
pub enum ResultState {
    #[default]
    Empty,
    Running,
    Error(String),
    Ok(QueryResult),
}

/// 排序方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

pub struct ResultPanel {
    pub(super) state: ResultState,
    /// 异步任务（如导出）完成后挂这里，下次 render 在 window 上下文里 push
    pub(super) pending_notification: Option<Notification>,
    /// 当前选中单元格 (row_idx, col_idx)，用于高亮 + 未来 ⌘C 复制
    pub(super) selected_cell: Option<(usize, usize)>,
    /// 多选行：表格首列 checkbox 勾选的行索引集合
    pub(super) selected_rows: BTreeSet<usize>,
    /// 当前结果对应的源 SQL（QueryTab 在 run/explain 后注入）
    pub(super) source_sql: Option<String>,
    /// 上游显式注入的目标 (schema, table)：表树点击时由 QueryPanel 传入
    pub(super) pinned_target: Option<(Option<String>, String)>,
    /// 列宽手动覆盖：用户拖动列分隔线后写入
    pub(super) col_width_overrides: Vec<Option<gpui::Pixels>>,
    /// 当前排序列与方向：单击列头切换 None→Asc→Desc→None
    pub(super) sort_by: Option<(usize, SortDir)>,
    /// 列过滤输入框：逗号分隔多列名（命中即显示该列）
    pub(super) column_filter_input: Entity<InputState>,
    /// 行过滤输入框：单一关键字
    pub(super) row_filter_input: Entity<InputState>,
    /// 单元格编辑弹框输入框：保活引用
    pub(super) cell_edit_input: Option<Entity<InputState>>,
    /// 行内编辑用的执行器（由 QueryTab 注入）
    pub(super) service: Option<Arc<ConnectionService>>,
    pub(super) connection: Option<ConnectionConfig>,
    /// 表元数据 cache（由 QueryTab 注入）：用于禁用视图上的写按钮
    pub(super) schema_cache: Option<Arc<RwLock<SchemaCache>>>,
    /// 新增草稿行：表格末尾追加可编辑空行
    pub(super) pending_insert: Option<PendingInsert>,
    /// 结果表格虚拟列表的垂直滚动句柄
    pub(super) uniform_scroll: UniformListScrollHandle,
    /// 外层水平滚动 div 的 ScrollHandle
    pub(super) h_scroll: ScrollHandle,
    /// 列过滤框的补全候选源
    pub(super) column_completion_source: Arc<RwLock<Vec<String>>>,
    /// SHOW WARNINGS 面板是否展开
    pub(super) warnings_expanded: bool,
}

impl ResultPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let column_completion_source: Arc<RwLock<Vec<String>>> = Arc::new(RwLock::new(Vec::new()));
        let provider = crate::sql_completion::ColumnFilterCompletionProvider::new_rc(
            column_completion_source.clone(),
        );
        let column_filter_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("过滤列（逗号分隔多列名）");
            state.lsp.completion_provider = Some(provider);
            state
        });
        let row_filter_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("过滤行（任意单元格包含）"));
        // 输入变化 → 触发 ResultPanel 重渲染
        cx.observe(&column_filter_input, |_, _, cx| cx.notify())
            .detach();
        cx.observe(&row_filter_input, |_, _, cx| cx.notify())
            .detach();

        Self {
            state: ResultState::Empty,
            pending_notification: None,
            selected_cell: None,
            source_sql: None,
            col_width_overrides: Vec::new(),
            sort_by: None,
            column_filter_input,
            row_filter_input,
            cell_edit_input: None,
            service: None,
            connection: None,
            schema_cache: None,
            pinned_target: None,
            selected_rows: BTreeSet::new(),
            pending_insert: None,
            uniform_scroll: UniformListScrollHandle::new(),
            h_scroll: ScrollHandle::new(),
            column_completion_source,
            warnings_expanded: false,
        }
    }

    pub(super) fn uniform_scroll(&self) -> &UniformListScrollHandle {
        &self.uniform_scroll
    }

    pub(super) fn h_scroll(&self) -> &ScrollHandle {
        &self.h_scroll
    }

    /// 进入新增模式：表格末尾追加可编辑草稿行（DataGrip 风格）
    pub fn start_insert(
        &mut self,
        columns: Vec<Column>,
        inputs: Vec<Entity<InputState>>,
        cx: &mut Context<Self>,
    ) {
        self.pending_insert = Some(PendingInsert { columns, inputs });
        let pending_idx = if let ResultState::Ok(qr) = &self.state {
            qr.rows.len().min(MAX_ROWS_DISPLAY)
        } else {
            0
        };
        self.uniform_scroll
            .scroll_to_item(pending_idx, ScrollStrategy::Center);
        cx.notify();
    }

    pub(crate) fn pending_insert(&self) -> Option<&PendingInsert> {
        self.pending_insert.as_ref()
    }

    pub(super) fn cancel_insert(&mut self, cx: &mut Context<Self>) {
        self.pending_insert = None;
        cx.notify();
    }

    /// 提交新增：遍历每列 InputState 校验后调 apply_insert_async
    pub(super) fn submit_insert(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_insert.as_ref() else {
            return;
        };
        let mut values: Vec<(String, Value)> = Vec::new();
        let mut err: Option<String> = None;
        for (col, input) in pending.columns.iter().zip(pending.inputs.iter()) {
            let text = input.read(cx).value().to_string();
            let nullable = col.nullable;
            let has_default = col.default_value.is_some() || col.is_primary_key;
            match parse_value_for_kind(col.data_type.kind, &text, nullable, has_default) {
                Ok(Some(v)) => values.push((col.name.clone(), v)),
                Ok(None) => {}
                Err(msg) => {
                    err = Some(format!("{}: {msg}", col.name));
                    break;
                }
            }
        }
        if let Some(msg) = err {
            self.pending_notification = Some(Notification::error(msg).autohide(true));
            cx.notify();
            return;
        }
        if values.is_empty() {
            self.pending_notification = Some(Notification::warning("请至少填一列").autohide(true));
            cx.notify();
            return;
        }
        self.apply_insert_async(values, cx);
        // 提交后立即退出草稿模式（apply 是异步的，结果通过 toast 反馈）
        self.pending_insert = None;
        cx.notify();
    }

    /// 上游（QueryTab.run）注入精确目标表，避免 SQL parse 误差
    pub fn set_pinned_target(&mut self, target: Option<(Option<String>, String)>) {
        self.pinned_target = target;
    }

    /// 当前结果集对应的目标表的引用字符串：优先用 pinned_target，再回退 SQL 解析
    pub(super) fn current_table_ref(&self) -> Option<String> {
        let driver = self.connection.as_ref().map(|c| c.driver)?;
        if let Some((schema, table)) = &self.pinned_target {
            return Some(match schema {
                Some(s) => format!(
                    "{}.{}",
                    driver.quote_identifier(s),
                    driver.quote_identifier(table)
                ),
                None => driver.quote_identifier(table),
            });
        }
        self.source_sql
            .as_deref()
            .and_then(|sql| extract_first_table_ref(sql, driver))
    }

    pub fn set_executor(
        &mut self,
        service: Option<Arc<ConnectionService>>,
        connection: Option<ConnectionConfig>,
    ) {
        self.service = service;
        self.connection = connection;
    }

    pub fn set_schema_cache(&mut self, cache: Option<Arc<RwLock<SchemaCache>>>) {
        self.schema_cache = cache;
    }

    /// 当前结果集对应的目标是否视图（视图禁止 INSERT/UPDATE/DELETE）
    pub(super) fn target_is_view(&self) -> bool {
        let Some(cache) = &self.schema_cache else {
            return false;
        };
        if let Some((schema, table)) = &self.pinned_target {
            return cache.read().is_view(schema.as_deref(), table);
        }
        let Some(sql) = self.source_sql.as_deref() else {
            return false;
        };
        let tables = crate::sql_completion::extract_tables_in_use_for_prefetch(sql);
        let Some((schema, table)) = tables.into_iter().next() else {
            return false;
        };
        cache.read().is_view(schema.as_deref(), &table)
    }

    pub(super) fn set_cell_edit_input(&mut self, input: Option<Entity<InputState>>) {
        self.cell_edit_input = input;
    }

    pub(super) fn cell_info(&self, ri: usize, ci: usize) -> Option<(String, String, bool)> {
        let ResultState::Ok(result) = &self.state else {
            return None;
        };
        let col_name = result.columns.get(ci)?.clone();
        let val = result.rows.get(ri)?.values.get(ci)?;
        let has_pk = find_pk_idx(result).is_some();
        Some((col_name, val.display_for_edit(), has_pk))
    }

    pub fn set_source_sql(&mut self, sql: Option<String>) {
        self.source_sql = sql;
    }

    pub fn set_state(&mut self, state: ResultState, cx: &mut Context<Self>) {
        match &state {
            ResultState::Ok(qr) => {
                *self.column_completion_source.write() = qr.columns.clone();
            }
            _ => {
                self.column_completion_source.write().clear();
            }
        }
        self.state = state;
        // 数据集变更后清除选中、排序、列宽覆盖、新增草稿
        self.selected_cell = None;
        self.selected_rows.clear();
        self.sort_by = None;
        self.col_width_overrides.clear();
        self.pending_insert = None;
        self.warnings_expanded = false;
        // 切表/重跑时双向归位：垂直回顶 + 水平回左
        self.uniform_scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.h_scroll.set_offset(Point::new(px(0.0), px(0.0)));
        cx.notify();
    }

    pub(super) fn selected_rows(&self) -> &BTreeSet<usize> {
        &self.selected_rows
    }

    pub(super) fn toggle_row_selected(&mut self, ri: usize, cx: &mut Context<Self>) {
        if !self.selected_rows.remove(&ri) {
            self.selected_rows.insert(ri);
        }
        cx.notify();
    }

    pub(super) fn toggle_all_rows(&mut self, total: usize, cx: &mut Context<Self>) {
        if self.selected_rows.len() == total {
            self.selected_rows.clear();
        } else {
            self.selected_rows = (0..total).collect();
        }
        cx.notify();
    }

    pub(super) fn set_col_width_override(&mut self, col_ix: usize, width: gpui::Pixels) {
        let n_cols = match &self.state {
            ResultState::Ok(r) => r.columns.len(),
            _ => return,
        };
        if self.col_width_overrides.len() != n_cols {
            self.col_width_overrides.resize(n_cols, None);
        }
        if col_ix < self.col_width_overrides.len() {
            self.col_width_overrides[col_ix] = Some(width);
        }
    }

    pub(super) fn col_width_override(&self, col_ix: usize) -> Option<gpui::Pixels> {
        self.col_width_overrides.get(col_ix).copied().flatten()
    }

    pub(super) fn toggle_sort(&mut self, col_idx: usize, cx: &mut Context<Self>) {
        self.sort_by = match self.sort_by {
            Some((ci, SortDir::Asc)) if ci == col_idx => Some((col_idx, SortDir::Desc)),
            Some((ci, SortDir::Desc)) if ci == col_idx => None,
            _ => Some((col_idx, SortDir::Asc)),
        };
        self.selected_cell = None;
        cx.notify();
    }

    pub(super) fn sort_by(&self) -> Option<(usize, SortDir)> {
        self.sort_by
    }

    pub(super) fn selected_cell(&self) -> Option<(usize, usize)> {
        self.selected_cell
    }

    pub(super) fn set_selected_cell(&mut self, cell: Option<(usize, usize)>) {
        self.selected_cell = cell;
    }

    pub(super) fn set_pending_notification(&mut self, n: Option<Notification>) {
        self.pending_notification = n;
    }

    pub fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.column_filter_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.row_filter_input
            .update(cx, |s, cx| s.set_value("", window, cx));
    }

    pub(super) fn column_filter_text(&self, cx: &gpui::App) -> String {
        self.column_filter_input.read(cx).value().trim().to_string()
    }

    pub(super) fn row_filter_text(&self, cx: &gpui::App) -> String {
        self.row_filter_input.read(cx).value().trim().to_string()
    }

    pub fn column_filter_entity(&self) -> &Entity<InputState> {
        &self.column_filter_input
    }
    pub fn row_filter_entity(&self) -> &Entity<InputState> {
        &self.row_filter_input
    }

    pub fn state(&self) -> &ResultState {
        &self.state
    }
}
