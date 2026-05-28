//! 结果展示：仿 MySQL 表格风格
//! - 顶部：过滤列 + 过滤行 + 行数 + 导出按钮
//! - 主体：uniform_list 行级虚拟化的表格（列头 + 行）
//! - 单元格点击 → 弹文档详情 dialog

mod flatten;
mod table;
mod toolbar;

use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement, Point, Render, ScrollHandle,
    SharedString, Styled, UniformListScrollHandle, Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Sizable as _, WindowExt as _,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use ramag_domain::entities::MongoQueryResult;
use serde_json::Value;

pub use flatten::FlatTable;

pub struct ResultPanel {
    pub(crate) result: Option<MongoQueryResult>,
    pub(crate) error: Option<String>,
    pub(crate) running: bool,
    /// 扁平化表格视图（result 变化时重算）
    pub(crate) table: Option<Arc<FlatTable>>,
    /// 工具栏：过滤列名（逗号分隔多列）
    pub(crate) column_filter: Entity<InputState>,
    /// 工具栏：过滤行（子串包含；任意单元格匹配即保留）
    pub(crate) row_filter: Entity<InputState>,
    /// 表格虚拟列表纵滚句柄（uniform_list 内部 Y）
    pub(crate) uniform_scroll: UniformListScrollHandle,
    /// 横滚句柄（外层 div X；与 dbclient::result_table 同模式）
    pub(crate) h_scroll: ScrollHandle,
    _subscriptions: Vec<gpui::Subscription>,
}

impl ResultPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let column_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("过滤列（逗号分隔多列名）"));
        let row_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("过滤行（任意单元格包含）"));

        let subs = vec![
            cx.subscribe(&column_filter, |_this, _, _e: &InputEvent, cx| cx.notify()),
            cx.subscribe(&row_filter, |_this, _, _e: &InputEvent, cx| cx.notify()),
        ];

        Self {
            result: None,
            error: None,
            running: false,
            table: None,
            column_filter,
            row_filter,
            uniform_scroll: UniformListScrollHandle::new(),
            h_scroll: ScrollHandle::new(),
            _subscriptions: subs,
        }
    }

    pub fn set_running(&mut self, cx: &mut Context<Self>) {
        self.running = true;
        self.error = None;
        cx.notify();
    }

    pub fn set_result(&mut self, r: MongoQueryResult, cx: &mut Context<Self>) {
        let table = if r.documents.is_empty() {
            None
        } else {
            Some(Arc::new(flatten::build_flat_table(&r.documents)))
        };
        self.table = table;
        self.result = Some(r);
        self.error = None;
        self.running = false;
        // 切结果时把横滚归位最左（与 dbclient::result_table 同款），避免新表格沿用旧的横滚 X 位置
        self.h_scroll.set_offset(Point::new(px(0.0), px(0.0)));
        cx.notify();
    }

    pub fn set_error(&mut self, err: String, cx: &mut Context<Self>) {
        self.error = Some(err);
        self.running = false;
        cx.notify();
    }

    /// 当前过滤后的列索引（None 表示全选）；空过滤串等价 None
    pub(crate) fn filtered_column_indices(&self, cx: &gpui::App) -> Option<Vec<usize>> {
        let raw = self.column_filter.read(cx).value().to_string();
        let q = raw.trim();
        if q.is_empty() {
            return None;
        }
        let table = self.table.as_ref()?;
        let wanted: Vec<String> = q
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        if wanted.is_empty() {
            return None;
        }
        let indices: Vec<usize> = table
            .columns
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                let lower = c.path.to_ascii_lowercase();
                wanted.iter().any(|w| lower.contains(w))
            })
            .map(|(i, _)| i)
            .collect();
        if indices.is_empty() {
            None
        } else {
            Some(indices)
        }
    }

    /// 当前过滤后的行索引；空过滤串等价 None
    pub(crate) fn filtered_row_indices(&self, cx: &gpui::App) -> Option<Vec<usize>> {
        let raw = self.row_filter.read(cx).value().to_string();
        let q = raw.trim().to_ascii_lowercase();
        if q.is_empty() {
            return None;
        }
        let table = self.table.as_ref()?;
        let indices: Vec<usize> = table
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.iter().any(|c| c.text.to_ascii_lowercase().contains(&q)))
            .map(|(i, _)| i)
            .collect();
        Some(indices)
    }

    /// 弹文档详情 dialog（保留 API，目前未使用；如需重新启用，把 table.rs cell on_click 改回行级别）
    #[allow(dead_code)]
    pub(crate) fn open_detail_dialog(
        &self,
        doc: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pretty = serde_json::to_string_pretty(&doc).unwrap_or_else(|_| doc.to_string());
        let input: Entity<InputState> = cx.new(|cx_inner| {
            InputState::new(window, cx_inner)
                .multi_line(true)
                .default_value(pretty)
        });
        window.open_dialog(cx, move |dialog, _w, _app| {
            let input = input.clone();
            dialog
                .title("文档详情")
                .close_button(true)
                .w(px(820.0))
                .p(px(20.0))
                .content(move |content, _, _| {
                    content.child(
                        div()
                            .w_full()
                            .h(px(540.0))
                            .child(Input::new(&input).small().h_full().disabled(true)),
                    )
                })
        });
    }

    /// 双击单元格 → 弹该单元格内容详情（与 MySQL dbclient::cell_edit_dialog 同款交互）。
    /// 标题是「{列名} ({类型})」；内容若是合法 JSON 自动 pretty 格式化
    pub(crate) fn open_cell_dialog(
        &self,
        column_path: String,
        kind: &'static str,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let display = if text.starts_with('{') || text.starts_with('[') {
            serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| serde_json::to_string_pretty(&v).ok())
                .unwrap_or_else(|| text.clone())
        } else {
            text.clone()
        };
        let title: SharedString = SharedString::from(format!("{column_path}  ({kind})"));
        let input: Entity<InputState> = cx.new(|cx_inner| {
            InputState::new(window, cx_inner)
                .multi_line(true)
                .default_value(display)
        });
        window.open_dialog(cx, move |dialog, _w, _app| {
            let input = input.clone();
            let title = title.clone();
            dialog
                .title(title)
                .close_button(true)
                .w(px(720.0))
                .p(px(20.0))
                .content(move |content, _, _| {
                    content.child(
                        div()
                            .w_full()
                            .h(px(400.0))
                            .child(Input::new(&input).small().h_full().disabled(true)),
                    )
                })
        });
    }
}

impl Render for ResultPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 把需要的颜色字段提前 Copy 出来，避免 cx.theme() 的 immut borrow 与 toolbar/table 的 mut borrow 冲突
        let bg = cx.theme().background;
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;

        if self.running {
            return v_flex()
                .size_full()
                .bg(bg)
                .child(toolbar::render(self, cx))
                .child(empty_hint("执行中…", muted))
                .into_any_element();
        }
        if let Some(err) = self.error.clone() {
            return v_flex()
                .size_full()
                .bg(bg)
                .child(toolbar::render(self, cx))
                .child(error_hint(err, danger))
                .into_any_element();
        }
        let Some(result) = self.result.clone() else {
            return v_flex()
                .size_full()
                .bg(bg)
                .child(toolbar::render(self, cx))
                .child(empty_hint(
                    "（点击左侧 collection 自动开 Tab，或在编辑器写命令后运行）",
                    muted,
                ))
                .into_any_element();
        };
        let Some(table_arc) = self.table.clone() else {
            let hint = if result.affected > 0 {
                format!("已执行写操作，影响 {} 条", result.affected)
            } else {
                "（无文档返回）".to_string()
            };
            return v_flex()
                .size_full()
                .bg(bg)
                .child(toolbar::render(self, cx))
                .child(empty_hint(hint, muted))
                .into_any_element();
        };

        let col_indices = self.filtered_column_indices(cx);
        let row_indices = self.filtered_row_indices(cx);

        // 底部 status bar：行数 + 耗时摘要（仿 dbclient::result_table）
        let total_docs = result.documents.len();
        let elapsed = result.elapsed_ms;
        let filtered_rows = row_indices.as_ref().map(|v| v.len()).unwrap_or(total_docs);
        let visible_cols_count = col_indices
            .as_ref()
            .map(|v| v.len())
            .unwrap_or_else(|| self.table.as_ref().map(|t| t.columns.len()).unwrap_or(0));
        let total_cols = self.table.as_ref().map(|t| t.columns.len()).unwrap_or(0);
        let summary = match (
            row_indices.is_some(),
            col_indices.is_some(),
            filtered_rows == total_docs,
        ) {
            (true, true, _) => format!(
                "命中 {visible_cols_count} / {total_cols} 列 · {filtered_rows} / {total_docs} 行 · 耗时 {elapsed}ms"
            ),
            (true, false, _) => format!(
                "命中 {filtered_rows} / {total_docs} 行 · 耗时 {elapsed}ms"
            ),
            (false, true, _) => format!(
                "命中 {visible_cols_count} / {total_cols} 列 · {total_docs} 行 · 耗时 {elapsed}ms"
            ),
            (false, false, _) => format!("{total_docs} 行 · 耗时 {elapsed}ms"),
        };

        v_flex()
            .size_full()
            .bg(bg)
            .child(toolbar::render(self, cx))
            .child(div().h(px(1.0)).bg(border))
            .child(div().flex_1().min_h_0().child(table::render(
                self,
                table_arc,
                col_indices,
                row_indices,
                cx,
            )))
            .child(render_status_bar(summary, border, muted, bg))
            .into_any_element()
    }
}

/// 底部 status bar：行数 / 耗时 / 过滤命中数（与 dbclient::result_table 同款）
fn render_status_bar(
    summary: String,
    border: gpui::Hsla,
    muted: gpui::Hsla,
    bg: gpui::Hsla,
) -> impl IntoElement {
    div()
        .id("mongo-status-bar")
        .w_full()
        .flex_none()
        .px(px(12.0))
        .py(px(4.0))
        .border_t_1()
        .border_color(border)
        .bg(bg)
        .text_xs()
        .text_color(muted)
        .child(SharedString::from(summary))
}

fn empty_hint(text: impl Into<SharedString>, color: gpui::Hsla) -> gpui::Stateful<gpui::Div> {
    div()
        .id("mongo-result-hint")
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .px(px(12.0))
        .py(px(10.0))
        .text_xs()
        .text_color(color)
        .child(text.into())
}

fn error_hint(text: String, color: gpui::Hsla) -> gpui::Stateful<gpui::Div> {
    div()
        .id("mongo-result-error")
        .flex_1()
        .px(px(12.0))
        .py(px(10.0))
        .text_xs()
        .text_color(color)
        .child(SharedString::from(text))
}
