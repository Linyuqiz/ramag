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
    AnyElement, ClickEvent, ClipboardItem, Context, DragMoveEvent, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement, Render, SharedString, Styled, div, prelude::*, px,
    uniform_list,
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
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputState},
    menu::ContextMenuExt as _,
    notification::Notification,
    v_flex,
};

use crate::actions::{CopyCellValue, CopySelectedColumn};
use ramag_domain::entities::{QueryResult, Row, Value};

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
#[allow(clippy::too_many_arguments)]
fn render_header_cell(
    ci: usize,
    columns: &[String],
    column_types: &[String],
    col_widths: &[gpui::Pixels],
    current_sort: Option<(usize, SortDir)>,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    border: gpui::Hsla,
    cx: &mut Context<ResultPanel>,
) -> AnyElement {
    let col = &columns[ci];
    let col_name = col.clone();
    let type_label: Option<SharedString> = column_types
        .get(ci)
        .filter(|s| !s.is_empty())
        .map(|s| SharedString::from(s.to_lowercase()));
    let sort_arrow: Option<&'static str> = match current_sort {
        Some((c, SortDir::Asc)) if c == ci => Some("▲"),
        Some((c, SortDir::Desc)) if c == ci => Some("▼"),
        _ => None,
    };
    let cw = col_widths[ci];
    div()
        .id(SharedString::from(format!("hdr-{ci}")))
        .w(cw)
        .min_w(cw)
        .max_w(cw)
        .flex_none()
        .border_r_1()
        .border_color(border)
        .overflow_hidden()
        .cursor_pointer()
        .relative()
        .on_click(cx.listener(move |this, e: &ClickEvent, _, cx| {
            if e.click_count() >= 2 {
                cx.write_to_clipboard(ClipboardItem::new_string(col_name.to_string()));
                this.set_pending_notification(Some(
                    Notification::success(format!("已复制列名 {col_name}")).autohide(true),
                ));
                cx.notify();
            } else {
                this.toggle_sort(ci, cx);
            }
        }))
        .child(
            h_flex()
                .w_full()
                .h_full()
                .px_3()
                .gap_1p5()
                .items_center()
                .overflow_hidden()
                .child(
                    div()
                        .min_w_0()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(SharedString::from(col.clone())),
                )
                .when_some(type_label, |this, t| {
                    this.child(
                        div()
                            .flex_none()
                            .text_xs()
                            .font_weight(gpui::FontWeight::NORMAL)
                            .text_color(muted_fg)
                            .whitespace_nowrap()
                            .child(t),
                    )
                })
                .when_some(sort_arrow, |this, a| {
                    this.child(div().flex_none().text_xs().text_color(muted_fg).child(a))
                }),
        )
        .child(render_col_resize_handle(ci, cx))
        .into_any_element()
}

/// 单行数据渲染：在 uniform_list closure 内被调
/// `frame` 是 Rc 共享数据（列宽 / 颜色 / mono 字体等不变量）
fn render_data_row(
    panel: &mut ResultPanel,
    frame: &TableRowFrame,
    idx: usize,
    cx: &mut Context<ResultPanel>,
) -> AnyElement {
    let row = &frame.display_rows[idx];
    let bg = if idx % 2 == 0 {
        frame.muted_bg.opacity(0.0)
    } else {
        frame.muted_bg.opacity(0.35)
    };
    let selected = panel.selected_cell();
    let selected_rows_set = panel.selected_rows().clone();
    let panel_entity = cx.entity();

    // 数据 cell
    let cells: Vec<AnyElement> = frame
        .visible_col_indices
        .iter()
        .map(|&ci| {
            let val = row.values.get(ci).cloned().unwrap_or(Value::Null);
            let display = val.display_preview(60);
            let is_null = matches!(val, Value::Null);
            let is_selected = selected == Some((idx, ci));
            let is_right = *frame.right_align.get(ci).unwrap_or(&false);
            let cw = frame.col_widths[ci];
            let row_idx = idx;
            let mono_font = frame.mono_font.clone();
            let fg = frame.fg;
            let muted_fg = frame.muted_fg;
            let border = frame.border;
            let accent = frame.accent;
            div()
                .id(SharedString::from(format!("cell-{idx}-{ci}")))
                .w(cw)
                .min_w(cw)
                .max_w(cw)
                .flex_none()
                .border_r_1()
                .border_color(border)
                .overflow_hidden()
                .cursor_pointer()
                .when(is_selected, |this| this.bg(accent.opacity(0.35)))
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    this.set_selected_cell(Some((row_idx, ci)));
                    if e.click_count() >= 2 {
                        open_cell_editor(this, row_idx, ci, window, cx);
                    }
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, _, _, cx| {
                        this.set_selected_cell(Some((row_idx, ci)));
                        cx.notify();
                    }),
                )
                .context_menu(|menu, _, _| {
                    menu.menu_with_icon("复制单元格", IconName::Copy, Box::new(CopyCellValue))
                        .menu_with_icon("复制列名", IconName::Copy, Box::new(CopySelectedColumn))
                })
                .child(
                    div()
                        .w_full()
                        .px_3()
                        .text_xs()
                        .font_family(mono_font)
                        .text_color(if is_null { muted_fg } else { fg })
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .when(is_right, |this| this.text_right())
                        .child(SharedString::from(display)),
                )
                .into_any_element()
        })
        .collect();

    // 行号
    let row_num_cell = div()
        .w(frame.row_num_width)
        .flex_none()
        .px_2()
        .text_xs()
        .font_family(frame.mono_font.clone())
        .text_color(frame.muted_fg)
        .text_right()
        .border_r_1()
        .border_color(frame.border)
        .child(SharedString::from((idx + 1).to_string()))
        .into_any_element();

    // 多选 checkbox
    let row_checkbox_cell = {
        let panel = panel_entity.clone();
        let row_idx = idx;
        let is_row_selected = selected_rows_set.contains(&idx);
        div()
            .w(frame.checkbox_col_width)
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(frame.border)
            .child(
                h_flex()
                    .w_full()
                    .h_full()
                    .items_center()
                    .justify_center()
                    .child(
                        Checkbox::new(SharedString::from(format!("row-cb-{idx}")))
                            .checked(is_row_selected)
                            .on_click(move |_: &bool, _, app| {
                                panel.update(app, |this, cx| {
                                    this.toggle_row_selected(row_idx, cx);
                                });
                            }),
                    ),
            )
            .into_any_element()
    };

    h_flex()
        .id(SharedString::from(format!("row-{idx}")))
        .w(frame.total_content_width)
        .h(px(32.0))
        .flex_none()
        .items_center()
        .bg(bg)
        .border_b_1()
        .border_color(frame.border)
        .child(row_checkbox_cell)
        .child(row_num_cell)
        .children(cells)
        .into_any_element()
}

/// 草稿插入行：作为 uniform_list 最后一项；高度同数据行 32px 保持等高
/// 不可勾选（checkbox 占位），行号位置用 "+" 标记
fn render_pending_row(
    panel: &mut ResultPanel,
    frame: &TableRowFrame,
    _cx: &mut Context<ResultPanel>,
) -> AnyElement {
    let Some(pending) = panel.pending_insert() else {
        return div().into_any_element();
    };
    let cb_cell = div()
        .w(frame.checkbox_col_width)
        .h_full()
        .flex_none()
        .border_r_1()
        .border_color(frame.border)
        .into_any_element();
    let num_cell = div()
        .w(frame.row_num_width)
        .flex_none()
        .px_2()
        .text_xs()
        .font_family(frame.mono_font.clone())
        .text_color(frame.accent)
        .text_right()
        .border_r_1()
        .border_color(frame.border)
        .child(SharedString::from("+"))
        .into_any_element();
    let mut input_cells: Vec<AnyElement> = Vec::with_capacity(frame.visible_col_indices.len());
    for &ci in &frame.visible_col_indices {
        let col_name_at = &frame.columns[ci];
        let cw = frame.col_widths[ci];
        let input = pending
            .columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(col_name_at))
            .and_then(|p| pending.inputs.get(p).cloned());
        let cell = div()
            .w(cw)
            .min_w(cw)
            .max_w(cw)
            .flex_none()
            .border_r_1()
            .border_color(frame.border)
            .px_1()
            .when_some(input, |this, state| {
                this.child(Input::new(&state).small().bordered(false))
            })
            .into_any_element();
        input_cells.push(cell);
    }
    h_flex()
        .id("row-pending")
        .w(frame.total_content_width)
        .h(px(32.0))
        .flex_none()
        .items_center()
        .bg(frame.accent.opacity(0.08))
        .border_b_1()
        .border_color(frame.border)
        .child(cb_cell)
        .child(num_cell)
        .children(input_cells)
        .into_any_element()
}

/// 按列头（含类型副标）+ 前 100 行内容估算列宽
/// 单元格用 mono 字体，每字符约 7.5px，加 24px 左右内边距 + 边框
/// 范围：[100, 380] px。DateTime 等长格式自然得到接近 280px。
fn estimate_col_width(
    ci: usize,
    columns: &[String],
    column_types: &[String],
    rows: &[Row],
) -> gpui::Pixels {
    const MIN_W: f32 = 100.0;
    const MAX_W: f32 = 380.0;
    const PER_CHAR: f32 = 7.5;
    const PADDING: f32 = 28.0;

    let col_chars = columns.get(ci).map(|s| s.chars().count()).unwrap_or(0);
    // 类型副标：列名 + gap(≈1 字符) + 类型字符；保证字段名永不被截断
    let type_chars = column_types
        .get(ci)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().count() + 1)
        .unwrap_or(0);
    let header_chars = col_chars + type_chars;

    let mut max_chars = header_chars;
    // display_preview(60) 与渲染保持一致：被截断成 60 的内容自然不会撑爆 380 上限
    for row in rows.iter().take(100) {
        if let Some(v) = row.values.get(ci) {
            let chars = v.display_preview(60).chars().count();
            if chars > max_chars {
                max_chars = chars;
            }
        }
    }
    let est = max_chars as f32 * PER_CHAR + PADDING;
    px(est.clamp(MIN_W, MAX_W))
}

/// 列宽拖拽 drag value：携带列索引（被拖动的列）
#[derive(Clone)]
pub(super) struct ColResizeDrag(pub usize);

impl Render for ColResizeDrag {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// 表头每列右边缘的拖拽 handle（4px 宽，cursor-col-resize）
fn render_col_resize_handle(ci: usize, cx: &mut Context<ResultPanel>) -> AnyElement {
    div()
        .id(SharedString::from(format!("col-resize-{ci}")))
        .absolute()
        .right_0()
        .top_0()
        .h(px(34.0))
        .w(px(4.0))
        .cursor_col_resize()
        .on_drag(ColResizeDrag(ci), |drag, _pos, _, cx| {
            cx.new(|_| drag.clone())
        })
        .on_drag_move(
            cx.listener(move |this, e: &DragMoveEvent<ColResizeDrag>, _, cx| {
                let drag = e.drag(cx);
                let mouse_x = e.event.position.x;
                let handle_right = e.bounds.right();
                let delta = mouse_x - handle_right;
                if delta == px(0.0) {
                    return;
                }
                let cur = this.col_width_override(drag.0).unwrap_or_else(|| px(180.0));
                let new_w = (cur + delta).max(px(60.0)).min(px(800.0));
                this.set_col_width_override(drag.0, new_w);
                cx.notify();
            }),
        )
        .into_any_element()
}

fn detect_numeric_column(ci: usize, rows: &[Row]) -> bool {
    let mut has_num = false;
    let mut all_num = true;
    for row in rows.iter().take(20) {
        if let Some(v) = row.values.get(ci) {
            match v {
                Value::Null => {}
                Value::Int(_) | Value::Float(_) => has_num = true,
                _ => {
                    all_num = false;
                    break;
                }
            }
        }
    }
    has_num && all_num
}

/// 同步打开单元格编辑弹框：必须在 listener 内调（已持 ResultPanel mut ref）
fn open_cell_editor(
    panel: &mut ResultPanel,
    ri: usize,
    ci: usize,
    window: &mut gpui::Window,
    cx: &mut Context<ResultPanel>,
) {
    let Some((col_name, initial_text, has_pk)) = panel.cell_info(ri, ci) else {
        return;
    };
    let input = cx.new(|cx_inner| {
        InputState::new(window, cx_inner)
            .multi_line(true)
            .rows(8)
            .default_value(initial_text)
    });
    panel.set_cell_edit_input(Some(input.clone()));
    let panel_entity = cx.entity();
    super::cell_edit_dialog::open(panel_entity, ri, ci, col_name, input, has_pk, window, cx);
}

/// 比较两个 Value：Null 视为最小，同型按值比较，跨型用字符串兜底
pub(super) fn compare_values(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, _) => Ordering::Less,
        (_, None) => Ordering::Greater,
        (Some(x), Some(y)) => match (x, y) {
            (Value::Null, Value::Null) => Ordering::Equal,
            (Value::Null, _) => Ordering::Less,
            (_, Value::Null) => Ordering::Greater,
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            (Value::Int(a), Value::Int(b)) => a.cmp(b),
            (Value::Float(a), Value::Float(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            (Value::Int(a), Value::Float(b)) => {
                (*a as f64).partial_cmp(b).unwrap_or(Ordering::Equal)
            }
            (Value::Float(a), Value::Int(b)) => {
                a.partial_cmp(&(*b as f64)).unwrap_or(Ordering::Equal)
            }
            (Value::Text(a), Value::Text(b)) => a.cmp(b),
            (Value::DateTime(a), Value::DateTime(b)) => a.cmp(b),
            (Value::Bytes(a), Value::Bytes(b)) => a.cmp(b),
            _ => x
                .display_preview(usize::MAX)
                .cmp(&y.display_preview(usize::MAX)),
        },
    }
}

/// Hsla 透明度便捷调用
pub(super) trait OpacityExt {
    fn opacity(self, alpha: f32) -> Self;
}

impl OpacityExt for gpui::Hsla {
    fn opacity(mut self, alpha: f32) -> Self {
        self.a = alpha.clamp(0.0, 1.0);
        self
    }
}
