//! 表格渲染：列头 + 行虚拟化（uniform_list）。
//! 单元格点击 → 弹该行完整文档详情；列头按 dotted path 显示类型短标签

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    Context, Hsla, InteractiveElement as _, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, px, uniform_list,
};
use gpui_component::{ActiveTheme, checkbox::Checkbox, h_flex, v_flex};

use super::ResultPanel;
use super::flatten::{Column, FlatTable};

/// 禁用 GPUI 单轴 scroll 的"另一方向劫持"，wheel 严格按方向消费（与 dbclient::result_table 同款）
trait RestrictScrollExt: Styled + Sized {
    fn restrict_scroll_to_axis(mut self) -> Self {
        self.style().restrict_scroll_to_axis = Some(true);
        self
    }
}
impl<T: Styled> RestrictScrollExt for T {}

/// 单元格固定宽度（简化版：不做动态估算）
const CELL_WIDTH: f32 = 200.0;
/// 单行高度（与 dbclient::result_table header 34 协调，行 32px 视觉密度接近）
const ROW_HEIGHT: f32 = 32.0;
/// 列头高度（与 dbclient::result_table 完全一致）
const HEADER_HEIGHT: f32 = 34.0;
/// 单元格预览最大字符数
const CELL_PREVIEW_MAX: usize = 80;
/// 行选择复选框列宽度（与 dbclient::result_table checkbox_col_width 一致）
const CHECKBOX_WIDTH: f32 = 32.0;

pub(super) fn render(
    panel: &mut ResultPanel,
    table: Arc<FlatTable>,
    col_indices: Option<Vec<usize>>,
    row_indices: Option<Vec<usize>>,
    cx: &mut Context<ResultPanel>,
) -> impl IntoElement {
    let border = cx.theme().border;
    let fg = cx.theme().foreground;
    let muted = cx.theme().muted_foreground;
    let secondary_bg = cx.theme().secondary;
    let mono_font = cx.theme().mono_font_family.clone();

    let visible_cols: Vec<usize> =
        col_indices.unwrap_or_else(|| (0..table.columns.len()).collect());
    let visible_rows: Vec<usize> = row_indices.unwrap_or_else(|| (0..table.rows.len()).collect());

    // 行号列宽：按总行数位数动态算（与 dbclient::result_table 同算法，clamp 40-70）
    let row_num_width =
        px((table.rows.len().to_string().len() as f32 * 9.0 + 16.0).clamp(40.0, 70.0));

    // 全选复选框：勾选 / 取消当前可见的全部行
    let all_data_idx = visible_rows.clone();
    let all_selected =
        !all_data_idx.is_empty() && all_data_idx.iter().all(|i| panel.is_row_selected(*i));
    let entity_for_all = cx.entity().clone();
    let header_checkbox = div()
        .w(px(CHECKBOX_WIDTH))
        .flex_none()
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .border_r_1()
        .border_color(border)
        .child(
            Checkbox::new("mongo-cb-all")
                .checked(all_selected)
                .on_click(move |_: &bool, _, app| {
                    entity_for_all.update(app, |this, cx| this.toggle_all(&all_data_idx, cx))
                }),
        )
        .into_any_element();

    let header_row = render_header(
        header_checkbox,
        row_num_width,
        &table.columns,
        &visible_cols,
        fg,
        muted,
        border,
        secondary_bg,
    );
    // 总宽 = 复选框列 + 数据列总宽 + 行号列（动态）
    let total_width = px(CHECKBOX_WIDTH + CELL_WIDTH * visible_cols.len() as f32) + row_num_width;

    // uniform_list 闭包内需要 'static，clone Arc + 索引向量
    let table_for_list = table.clone();
    let cols_for_list = visible_cols.clone();
    let rows_for_list = visible_rows.clone();
    let docs_for_list: Arc<Vec<serde_json::Value>> = panel
        .result
        .as_ref()
        .map(|r| Arc::new(r.documents.clone()))
        .unwrap_or_else(|| Arc::new(Vec::new()));

    let body = uniform_list(
        "mongo-result-rows",
        rows_for_list.len(),
        cx.processor(move |panel, range: Range<usize>, _w, cx| {
            let theme = cx.theme();
            let fg = theme.foreground;
            let muted = theme.muted_foreground;
            let border = theme.border;
            let muted_bg = theme.muted;
            let mono = mono_font.clone();
            range
                .map(|i| {
                    let row_idx = rows_for_list[i];
                    let row = &table_for_list.rows[row_idx];
                    let doc = docs_for_list
                        .get(row_idx)
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let selected = panel.is_row_selected(row_idx);
                    let entity_for_row = cx.entity().clone();
                    let checkbox = div()
                        .w(px(CHECKBOX_WIDTH))
                        .flex_none()
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .border_r_1()
                        .border_color(border)
                        .child(
                            Checkbox::new(SharedString::from(format!("mongo-cb-{i}")))
                                .checked(selected)
                                .on_click(move |_: &bool, _, app| {
                                    entity_for_row
                                        .update(app, |this, cx| this.toggle_row(row_idx, cx))
                                }),
                        )
                        .into_any_element();
                    render_row(
                        checkbox,
                        row_num_width,
                        i,
                        row_idx,
                        row,
                        &cols_for_list,
                        &table_for_list.columns,
                        doc,
                        fg,
                        muted,
                        border,
                        muted_bg,
                        mono.clone(),
                        cx,
                    )
                })
                .collect::<Vec<_>>()
        }),
    )
    .track_scroll(&panel.uniform_scroll)
    .w(total_width)
    .flex_1()
    // list 仅 Y 滚，wheel dx 留给外层 div 消费（与 dbclient::result_table 同模式）
    .restrict_scroll_to_axis();

    // 嵌套 viewport：外层 div 处理 X 滚动，内层 uniform_list 处理 Y 虚拟化纵滚
    // - 外层 div: flex_1 + min_h_0/min_w_0 + overflow_x_scroll + restrict_axis + track_scroll(h_scroll)
    // - 内层 v_flex: w(total_width) + h_full（避免 size_full 重置 width）+ header + body
    v_flex().size_full().min_w_0().child(
        div()
            .id("mongo-table-h-scroll")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_x_scroll()
            .restrict_scroll_to_axis()
            .track_scroll(&panel.h_scroll)
            .child(
                v_flex()
                    .w(total_width)
                    .h_full()
                    .child(header_row.flex_none())
                    .child(body),
            ),
    )
}

#[allow(clippy::too_many_arguments)]
fn render_header(
    checkbox: gpui::AnyElement,
    row_num_width: gpui::Pixels,
    columns: &[Column],
    visible_cols: &[usize],
    fg: Hsla,
    muted: Hsla,
    border: Hsla,
    bg: Hsla,
) -> gpui::Div {
    // 行号列占位（与数据行的「#」列对齐）
    let row_num_cell = div()
        .w(row_num_width)
        .flex_none()
        .h_full()
        .border_r_1()
        .border_color(border);

    let mut row = h_flex()
        .h(px(HEADER_HEIGHT))
        .flex_none()
        .items_center()
        .bg(bg)
        .border_b_1()
        .border_color(border)
        .child(checkbox)
        .child(row_num_cell);
    for &ci in visible_cols {
        let col = &columns[ci];
        let path = col.path.clone();
        let kind = col.kind;
        row = row.child(
            h_flex()
                .w(px(CELL_WIDTH))
                .flex_none()
                .h_full()
                .px_3()
                .gap_1p5()
                .items_center()
                .border_r_1()
                .border_color(border)
                .text_xs()
                .overflow_hidden()
                .child(
                    div()
                        .min_w_0()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(SharedString::from(path)),
                )
                .child(
                    div()
                        .flex_none()
                        .font_weight(gpui::FontWeight::NORMAL)
                        .text_color(muted)
                        .whitespace_nowrap()
                        .child(SharedString::from(kind)),
                ),
        );
    }
    row
}

#[allow(clippy::too_many_arguments)]
fn render_row(
    checkbox: gpui::AnyElement,
    row_num_width: gpui::Pixels,
    row_idx_in_view: usize,
    row_idx_in_data: usize,
    cells: &[super::flatten::Cell],
    visible_cols: &[usize],
    columns: &[Column],
    doc: serde_json::Value,
    fg: Hsla,
    muted: Hsla,
    border: Hsla,
    muted_bg: Hsla,
    mono_font: SharedString,
    cx: &mut Context<ResultPanel>,
) -> gpui::AnyElement {
    // 斑马纹：偶数行透明，奇数行 muted_bg 35% 透明度（与 dbclient::result_table 一致）
    let stripe = if row_idx_in_view.is_multiple_of(2) {
        muted_bg.opacity(0.0)
    } else {
        muted_bg.opacity(0.35)
    };

    // 行号列：满格高度（竖线连续）+ 等宽字体 + 右对齐
    let row_num_cell = div()
        .w(row_num_width)
        .flex_none()
        .h_full()
        .px_2()
        .text_xs()
        .font_family(mono_font.clone())
        .text_color(muted)
        .border_r_1()
        .border_color(border)
        .flex()
        .items_center()
        .justify_end()
        .child(SharedString::from((row_idx_in_data + 1).to_string()));

    let mut row = h_flex()
        .id(SharedString::from(format!("mongo-row-{row_idx_in_view}")))
        .h(px(ROW_HEIGHT))
        .items_center()
        .bg(stripe)
        .border_b_1()
        .border_color(border)
        .cursor_pointer()
        .child(checkbox)
        .child(row_num_cell);

    // 文档 _id：双击单元格编辑时用它作 update_one 的定位条件
    let row_id = doc.get("_id").cloned();
    for &ci in visible_cols {
        let cell = &cells[ci];
        let column = &columns[ci];
        let preview = truncate(&cell.text, CELL_PREVIEW_MAX);
        let is_null = cell.kind == "null" && preview.is_empty();
        // 数字类型列右对齐（与 dbclient is_right 同款）
        let is_right = matches!(column.kind, "int" | "double" | "decimal");
        let mf = mono_font.clone();
        // 捕获列信息 + 单元格全值，双击 → 弹单元格 dialog（与 dbclient 单元格编辑器同款交互）
        let path_for_click = column.path.clone();
        let kind_for_click = column.kind;
        let text_for_click = cell.text.clone();
        row = row.child(
            div()
                .id(SharedString::from(format!(
                    "mongo-cell-{row_idx_in_view}-{ci}"
                )))
                .w(px(CELL_WIDTH))
                .flex_none()
                .h_full()
                .border_r_1()
                .border_color(border)
                .overflow_hidden()
                .cursor_pointer()
                .on_click({
                    let id_for_click = row_id.clone();
                    cx.listener(move |panel, e: &gpui::ClickEvent, window, cx| {
                        if e.click_count() < 2 {
                            return;
                        }
                        // 可写 + 行有 _id → 编辑（update_one）；否则只读查看
                        match &id_for_click {
                            Some(id) if panel.can_write() => panel.open_cell_edit_dialog(
                                id.clone(),
                                path_for_click.clone(),
                                text_for_click.clone(),
                                window,
                                cx,
                            ),
                            _ => panel.open_cell_dialog(
                                path_for_click.clone(),
                                kind_for_click,
                                text_for_click.clone(),
                                window,
                                cx,
                            ),
                        }
                    })
                })
                .child(
                    div()
                        .w_full()
                        .h_full()
                        .px_3()
                        .flex()
                        .items_center()
                        .when(is_right, |this| this.justify_end())
                        .text_xs()
                        .font_family(mf)
                        .text_color(if is_null { muted } else { fg })
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(SharedString::from(if is_null {
                            "NULL".to_string()
                        } else {
                            preview
                        })),
                ),
        );
    }
    row.into_any_element()
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len).collect();
        format!("{truncated}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_keeps_short_string() {
        assert_eq!(truncate("hi", 10), "hi");
    }

    #[test]
    fn truncate_adds_ellipsis_for_long() {
        let s = truncate("abcdefghijklmnop", 5);
        assert_eq!(s, "abcde…");
    }
}
