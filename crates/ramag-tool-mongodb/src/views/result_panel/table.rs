//! 表格渲染：列头 + 行虚拟化（uniform_list）。
//! 单元格点击 → 弹该行完整文档详情；列头按 dotted path 显示类型短标签

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    Context, Hsla, InteractiveElement as _, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, px, uniform_list,
};
use gpui_component::{ActiveTheme, checkbox::Checkbox, h_flex, v_flex};

use super::flatten::{Column, FlatTable};
use super::{ResultPanel, SortDir};

/// 禁用 GPUI 单轴 scroll 的"另一方向劫持"，wheel 严格按方向消费（与 dbclient::result_table 同款）
trait RestrictScrollExt: Styled + Sized {
    fn restrict_scroll_to_axis(mut self) -> Self {
        self.style().restrict_scroll_to_axis = Some(true);
        self
    }
}
impl<T: Styled> RestrictScrollExt for T {}

/// 单元格固定宽度（简化版：不做动态估算）
pub(super) const CELL_WIDTH: f32 = 200.0;
/// 单行高度（与 dbclient::result_table header 34 协调，行 32px 视觉密度接近）
pub(super) const ROW_HEIGHT: f32 = 32.0;
/// 列头高度（与 dbclient::result_table 完全一致）
const HEADER_HEIGHT: f32 = 34.0;
/// 单元格预览最大字符数
pub(super) const CELL_PREVIEW_MAX: usize = 80;
/// 行选择复选框列宽度（与 dbclient::result_table checkbox_col_width 一致）
const CHECKBOX_WIDTH: f32 = 32.0;

pub(super) fn render(
    panel: &mut ResultPanel,
    table: Arc<FlatTable>,
    col_indices: Option<Vec<usize>>,
    row_indices: Option<Vec<usize>>,
    docs_override: Option<Arc<Vec<serde_json::Value>>>,
    allow_edit: bool,
    cx: &mut Context<ResultPanel>,
) -> impl IntoElement {
    let border = cx.theme().border;
    let fg = cx.theme().foreground;
    let muted = cx.theme().muted_foreground;
    let secondary_bg = cx.theme().secondary;
    let mono_font = cx.theme().mono_font_family.clone();

    let visible_cols: Vec<usize> =
        col_indices.unwrap_or_else(|| (0..table.columns.len()).collect());
    let mut visible_rows: Vec<usize> =
        row_indices.unwrap_or_else(|| (0..table.rows.len()).collect());
    // 排序：按 sort 列 path 定位列后对可见行重排（普通 / 钻取视图共用此函数；path 失配则不排）
    if let Some((sort_path, dir)) = panel.sort_by.clone()
        && let Some(si) = table.columns.iter().position(|c| c.path == sort_path)
    {
        let numeric = matches!(table.columns[si].kind, "int" | "double" | "decimal");
        visible_rows.sort_by(|&a, &b| {
            let ord = compare_cells(&table.rows[a][si].text, &table.rows[b][si].text, numeric);
            if matches!(dir, SortDir::Desc) {
                ord.reverse()
            } else {
                ord
            }
        });
    }

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
        panel.sort_by.clone(),
        fg,
        muted,
        border,
        secondary_bg,
        cx,
    );
    // 总宽 = 复选框列 + 数据列总宽 + 行号列（动态）
    let total_width = px(CHECKBOX_WIDTH + CELL_WIDTH * visible_cols.len() as f32) + row_num_width;

    // uniform_list 闭包内需要 'static，clone Arc + 索引向量
    let table_for_list = table.clone();
    let cols_for_list = visible_cols.clone();
    let rows_for_list = visible_rows.clone();
    // 行文档来源：默认当前结果集；展平视图等传 docs_override 覆盖（双击下钻据此取原值）
    let docs_for_list: Arc<Vec<serde_json::Value>> = docs_override.unwrap_or_else(|| {
        panel
            .result
            .as_ref()
            .map(|r| Arc::new(r.documents.clone()))
            .unwrap_or_else(|| Arc::new(Vec::new()))
    });

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
                    super::row::render_row(
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
                        allow_edit,
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
    current_sort: Option<(String, SortDir)>,
    fg: Hsla,
    muted: Hsla,
    border: Hsla,
    bg: Hsla,
    cx: &mut Context<ResultPanel>,
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
        // 排序箭头：当前排序列显示 ▲（升）/▼（降）
        let arrow: Option<&'static str> = match &current_sort {
            Some((p, SortDir::Asc)) if *p == path => Some("▲"),
            Some((p, SortDir::Desc)) if *p == path => Some("▼"),
            _ => None,
        };
        let path_for_click = path.clone();
        row = row.child(
            h_flex()
                .id(SharedString::from(format!("mongo-hdr-{ci}")))
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
                .cursor_pointer()
                // 单击列头切换该列排序（按列 path，钻取视图同样生效）
                .on_click(cx.listener(move |panel, _: &gpui::ClickEvent, _, cx| {
                    panel.toggle_sort(path_for_click.clone(), cx)
                }))
                .child(
                    div()
                        .min_w_0()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(SharedString::from(sanitize_inline(&path))),
                )
                .child(
                    div()
                        .flex_none()
                        .font_weight(gpui::FontWeight::NORMAL)
                        .text_color(muted)
                        .whitespace_nowrap()
                        .child(SharedString::from(kind)),
                )
                .when_some(arrow, |this, a| {
                    this.child(div().flex_none().text_color(muted).child(a))
                }),
        );
    }
    row
}

/// 单元格排序比较：空值（null）排前；数字列按数值，否则按字符串（ISO 日期 / oid 字典序合理）
fn compare_cells(a: &str, b: &str, numeric: bool) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a.is_empty(), b.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }
    if numeric && let (Ok(x), Ok(y)) = (a.parse::<f64>(), b.parse::<f64>()) {
        return x.partial_cmp(&y).unwrap_or(Ordering::Equal);
    }
    a.cmp(b)
}

pub(super) fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len).collect();
        format!("{truncated}…")
    }
}

/// 单行预览清洗：换行符（\n / \r）替换为空格。
/// GPUI 单行文本 shaping 断言不允许 \n（含 \n 直接 panic→abort）；仅用于表格显示文本，
/// 不改 cell.text 原值。无换行时零拷贝走 to_string，避免多余分配
pub(super) fn sanitize_inline(s: &str) -> String {
    if s.contains(['\n', '\r']) {
        s.replace(['\n', '\r'], " ")
    } else {
        s.to_string()
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

    #[test]
    fn sanitize_inline_strips_newlines() {
        // \n / \r / \r\n 均替换为空格，结果不含任何换行（否则 GPUI 渲染 panic）
        assert_eq!(sanitize_inline("a\nb"), "a b");
        assert_eq!(sanitize_inline("a\rb"), "a b");
        let s = sanitize_inline("x\ny\r\nz");
        assert!(!s.contains('\n') && !s.contains('\r'));
    }

    #[test]
    fn sanitize_inline_keeps_plain_text() {
        assert_eq!(sanitize_inline("plain text"), "plain text");
    }

    #[test]
    fn compare_cells_numeric_vs_text() {
        use std::cmp::Ordering;
        assert_eq!(compare_cells("9", "10", true), Ordering::Less); // 数值 9 < 10
        assert_eq!(compare_cells("9", "10", false), Ordering::Greater); // 字典序 "9" > "10"
        assert_eq!(compare_cells("", "x", false), Ordering::Less); // null 排前
        assert_eq!(compare_cells("x", "", false), Ordering::Greater);
    }
}
