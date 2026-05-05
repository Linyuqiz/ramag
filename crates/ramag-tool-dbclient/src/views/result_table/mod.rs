//! 结果集表格渲染（从 result_panel.rs 拆出，避免单文件过大）
//!
//! 行级虚拟化：用 GPUI `uniform_list` 仅渲染屏幕可见行，理论支持百万级
//! 行不卡（实际受 driver LIMIT 与 MAX_ROWS_DISPLAY 控制）。
//!
//! 渲染拆分：
//! - `render_table`：主入口，构建帧级 `TableRowFrame`（Rc 共享给 list closure）
//! - `render_data_row`：单行数据 cell + 行号 + checkbox（在 list closure 内被调）
//! - `render_pending_row`：草稿插入行（作为 list 最后一项；高度同数据行 32px）
//! - 其余 helper：列宽估算、数值列检测、排序比较等

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement, SharedString, Styled,
    div, prelude::*, px, uniform_list,
};

/// 关闭 GPUI 单轴 scroll 元素的"另一方向劫持"行为
///
/// GPUI 默认：overflow.x=Scroll 且 overflow.y!=Scroll 时，wheel 的 dy 会被自动当成 dx
/// 应用（反之亦然），结果是"往下滚 → 横向滚到底"或"往右滑 → 垂直滚到底"。
/// 设置 `restrict_scroll_to_axis = true` 禁用这个适配，wheel 严格按方向消费。
trait RestrictScrollExt: Styled + Sized {
    fn restrict_scroll_to_axis(mut self) -> Self {
        self.style().restrict_scroll_to_axis = Some(true);
        self
    }
}
impl<T: Styled> RestrictScrollExt for T {}
use gpui_component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex, v_flex,
};

use ramag_domain::entities::{QueryResult, Row};

use super::result_panel::{MAX_ROWS_DISPLAY, ResultPanel, SortDir};

/// 帧级数据：本次 render_table 计算一次，供 uniform_list closure 共享访问
/// 用 Rc 包装才能在 'static + Fn 闭包内 capture（不能 borrow 栈局部变量）
struct TableRowFrame {
    columns: Vec<String>,
    display_rows: Vec<Row>,
    visible_col_indices: Vec<usize>,
    col_widths: Vec<gpui::Pixels>,
    right_align: Vec<bool>,
    row_num_width: gpui::Pixels,
    checkbox_col_width: gpui::Pixels,
    total_content_width: gpui::Pixels,
    mono_font: SharedString,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    border: gpui::Hsla,
    muted_bg: gpui::Hsla,
    accent: gpui::Hsla,
}

/// 渲染单次查询结果表格
///
/// 入口由 ResultPanel::render 调用，接收所有需要的主题色和上下文
#[allow(clippy::too_many_arguments)]
pub(super) fn render_table(
    panel: &ResultPanel,
    result: QueryResult,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    secondary_bg: gpui::Hsla,
    border: gpui::Hsla,
    muted_bg: gpui::Hsla,
    accent: gpui::Hsla,
    cx: &mut Context<ResultPanel>,
) -> AnyElement {
    let result = &result;
    let columns = result.columns.clone();
    let column_types = result.column_types.clone();
    let total_rows = result.rows.len();
    let mut display_rows = result
        .rows
        .iter()
        .take(MAX_ROWS_DISPLAY)
        .cloned()
        .collect::<Vec<_>>();
    let truncated = total_rows > MAX_ROWS_DISPLAY;
    let affected = result.affected_rows;
    let elapsed = result.elapsed_ms;

    // 排序（仅排前 MAX_ROWS_DISPLAY 行）
    if let Some((sort_col, dir)) = panel.sort_by() {
        display_rows.sort_by(|a, b| {
            let av = a.values.get(sort_col);
            let bv = b.values.get(sort_col);
            let ord = compare_values(av, bv);
            if matches!(dir, SortDir::Desc) {
                ord.reverse()
            } else {
                ord
            }
        });
    }

    // 列 + 行过滤
    let col_filter = panel.column_filter_text(cx);
    let row_filter = panel.row_filter_text(cx).to_lowercase();
    let col_tokens: Vec<String> = col_filter
        .split(',')
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    let cols_filtering = !col_tokens.is_empty();
    let visible_col_indices: Vec<usize> = if cols_filtering {
        columns
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                let lc = c.to_lowercase();
                col_tokens.iter().any(|t| lc.contains(t))
            })
            .map(|(i, _)| i)
            .collect()
    } else {
        (0..columns.len()).collect()
    };
    let cols_filtered = cols_filtering;
    let total_cols = columns.len();
    let visible_cols_count = visible_col_indices.len();
    let pre_filter_count = display_rows.len();
    let row_filtering = !row_filter.is_empty();
    if row_filtering {
        let needle = row_filter.clone();
        let scoped_indices = visible_col_indices.clone();
        display_rows.retain(|row| {
            scoped_indices.iter().any(|&ci| {
                row.values
                    .get(ci)
                    .map(|v| {
                        v.display_preview(usize::MAX)
                            .to_lowercase()
                            .contains(&needle)
                    })
                    .unwrap_or(false)
            })
        });
    }
    let visible_count = display_rows.len();

    // DML/DDL：没有列，只显示 affected_rows
    if columns.is_empty() {
        return v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                div()
                    .text_lg()
                    .text_color(fg)
                    .child(format!("✓ {affected} 行受影响")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted_fg)
                    .child(format!("{elapsed} ms")),
            )
            .into_any_element();
    }

    // 注：0 行不再 early return；让 header + 空 body + 状态栏正常渲染，
    // 用户能看到列头与列类型，避免"查无结果"占位遮蔽元信息

    // 列宽 / 行号宽 / 总宽
    let col_widths: Vec<gpui::Pixels> = (0..columns.len())
        .map(|ci| {
            panel
                .col_width_override(ci)
                .unwrap_or_else(|| estimate_col_width(ci, &columns, &column_types, &display_rows))
        })
        .collect();
    let row_num_width = px((total_rows.to_string().len() as f32 * 9.0 + 16.0).clamp(40.0, 70.0));
    let checkbox_col_width = px(32.0);
    let total_content_width = visible_col_indices
        .iter()
        .map(|&ci| col_widths[ci])
        .fold(row_num_width + checkbox_col_width, |acc, w| acc + w);

    // 数据 cell 用 mono 字体（长 ID / 时间戳纵向对齐）；表头不用
    let mono_font = cx.theme().mono_font_family.clone();
    // 数值列检测：扫前 20 行，全是 Int/Float（允许 Null）→ 右对齐
    let right_align: Vec<bool> = (0..columns.len())
        .map(|ci| detect_numeric_column(ci, &display_rows))
        .collect();

    // ===== Header =====
    let current_sort = panel.sort_by();
    let header_cells: Vec<AnyElement> = visible_col_indices
        .iter()
        .map(|&ci| {
            render_header_cell(
                ci,
                &columns,
                &column_types,
                &col_widths,
                current_sort,
                fg,
                muted_fg,
                border,
                cx,
            )
        })
        .collect();

    let row_num_header = div()
        .w(row_num_width)
        .flex_none()
        .px_2()
        .border_r_1()
        .border_color(border)
        .into_any_element();

    let selected_rows_set = panel.selected_rows().clone();
    let visible_count_total = display_rows.len();
    let all_selected = visible_count_total > 0 && selected_rows_set.len() == visible_count_total;
    let panel_entity = cx.entity();

    let checkbox_header = {
        let panel = panel_entity.clone();
        div()
            .w(checkbox_col_width)
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(border)
            .child(
                h_flex()
                    .w_full()
                    .h_full()
                    .items_center()
                    .justify_center()
                    .child(
                        Checkbox::new("rows-toggle-all")
                            .checked(all_selected)
                            .on_click(move |_: &bool, _, app| {
                                panel.update(app, |this, cx| {
                                    this.toggle_all_rows(visible_count_total, cx);
                                });
                            }),
                    ),
            )
            .into_any_element()
    };

    let header = h_flex()
        .w(total_content_width)
        .h(px(34.0))
        .flex_none()
        .items_center()
        .bg(secondary_bg)
        .border_b_1()
        .border_color(border)
        .child(checkbox_header)
        .child(row_num_header)
        .children(header_cells);

    // ===== Body：uniform_list 行级虚拟化 =====
    // 把 row 渲染需要的不变数据装进 frame，Rc 共享给 closure（满足 'static + Fn）
    let frame = Rc::new(TableRowFrame {
        columns: columns.clone(),
        display_rows: display_rows.clone(),
        visible_col_indices: visible_col_indices.clone(),
        col_widths: col_widths.clone(),
        right_align,
        row_num_width,
        checkbox_col_width,
        total_content_width,
        mono_font,
        fg,
        muted_fg,
        border,
        muted_bg,
        accent,
    });

    let has_pending = panel.pending_insert().is_some();
    let row_count = frame.display_rows.len() + if has_pending { 1 } else { 0 };

    let frame_for_rows = frame.clone();
    let body = uniform_list(
        "result-rows",
        row_count,
        cx.processor(move |this, range: Range<usize>, _w, cx| {
            range
                .map(|i| {
                    if i < frame_for_rows.display_rows.len() {
                        render_data_row(this, &frame_for_rows, i, cx)
                    } else {
                        render_pending_row(this, &frame_for_rows, cx)
                    }
                })
                .collect::<Vec<_>>()
        }),
    )
    .track_scroll(panel.uniform_scroll())
    .w(frame.total_content_width)
    .flex_1()
    // 禁止 list 把 wheel dx 当 dy 用（list 是单 Y 滚，否则 dx 会被劫持垂直滚）
    .restrict_scroll_to_axis();

    // ===== Status Bar =====
    let selected_info: Option<String> = panel.selected_cell().and_then(|(ri, ci)| {
        let col_name = columns.get(ci)?.clone();
        let val = display_rows.get(ri)?.values.get(ci)?;
        let preview = val.display_preview(40);
        Some(format!("· [{}, {}] = {}", ri + 1, col_name, preview))
    });

    let status_bar = h_flex()
        .w_full()
        .flex_none()
        .items_center()
        .px_3()
        .py_1()
        .gap_2()
        .border_t_1()
        .border_color(border)
        .bg(secondary_bg)
        .text_xs()
        .text_color(muted_fg)
        .child(match (cols_filtered, row_filtering) {
            (true, true) => div().child(format!(
                "命中 {visible_cols_count} / {total_cols} 列 · {visible_count} / {pre_filter_count} 行"
            )),
            (true, false) => div().child(format!(
                "命中 {visible_cols_count} / {total_cols} 列 · {pre_filter_count} 行"
            )),
            (false, true) => {
                div().child(format!("命中 {visible_count} / {pre_filter_count} 行"))
            }
            (false, false) if truncated => div().child(format!(
                "显示 {MAX_ROWS_DISPLAY} / {total_rows} 行（已截断）"
            )),
            (false, false) => div().child(format!("{total_rows} 行")),
        })
        .child(div().child(format!("· 耗时 {elapsed} ms")))
        .when_some(selected_info, |this, info| {
            this.child(div().overflow_hidden().text_ellipsis().child(info))
        })
        .when(has_pending, |this| {
            let panel_for_cancel = panel_entity.clone();
            let panel_for_submit = panel_entity.clone();
            this.child(div().flex_1())
                .child(
                    Button::new("insert-cancel-bar")
                        .ghost()
                        .small()
                        .label("取消")
                        .on_click(move |_, _, app| {
                            panel_for_cancel.update(app, |r, cx| r.cancel_insert(cx));
                        }),
                )
                .child(
                    Button::new("insert-submit-bar")
                        .primary()
                        .small()
                        .label("提交")
                        .on_click(move |_, _, app| {
                            panel_for_submit.update(app, |r, cx| r.submit_insert(cx));
                        }),
                )
        });

    // 外层布局：v_flex 主轴；水平滚动由外层 div 处理，垂直虚拟化由 list 处理
    // 关键：
    // 1) 外层 div 用 overflow_x_scroll（仅 X），list 用 track_scroll 管 Y；
    //    wheel 事件先到 list 消费 Y delta，剩余 X 冒泡给 div 消费 X delta —— 嵌套
    //    viewport 标准行为，触控板含 Y 噪声时 list 也会少量滚动 Y
    // 2) 外层 div 通过 panel.h_scroll() 关联 ScrollHandle，跨 render 保持水平位置；
    //    切表时由 set_state 调 set_offset 主动归位左侧
    // 3) 内层 v_flex 用 h_full 而非 size_full —— size_full 含 w_full 会重置 width
    v_flex()
        .size_full()
        .min_w_0()
        .child(
            div()
                .id("result-h-scroll")
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_x_scroll()
                // 禁止外层 div 把 wheel dy 当 dx 用（div 是单 X 滚，否则 dy 会被劫持横向滚）
                .restrict_scroll_to_axis()
                .track_scroll(panel.h_scroll())
                .child(
                    v_flex()
                        .w(frame.total_content_width)
                        .h_full()
                        .child(header)
                        .child(body),
                ),
        )
        .child(status_bar)
        .into_any_element()
}

/// Header 单元格：列名（强）+ 类型副标（弱）+ 排序箭头（弱）+ 列宽拖拽 handle
mod cells;
mod helpers;

use cells::{render_data_row, render_header_cell, render_pending_row};
use helpers::{compare_values, detect_numeric_column, estimate_col_width};
