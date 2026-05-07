//! 表格 cell：表头 / 数据行 / 草稿插入行

use gpui::{
    AnyElement, ClickEvent, ClipboardItem, Context, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement, SharedString, Styled, div, prelude::*, px,
};
use gpui_component::{
    IconName, Sizable as _, checkbox::Checkbox, h_flex, input::Input, menu::ContextMenuExt as _,
    notification::Notification,
};
use ramag_domain::entities::Value;

use super::TableRowFrame;
use super::helpers::{OpacityExt as _, open_cell_editor, render_col_resize_handle};
use crate::actions::{CopyCellValue, CopySelectedColumn};
use crate::views::result_panel::{ResultPanel, SortDir};

#[allow(clippy::too_many_arguments)]
pub(super) fn render_header_cell(
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
pub(super) fn render_data_row(
    panel: &mut ResultPanel,
    frame: &TableRowFrame,
    idx: usize,
    cx: &mut Context<ResultPanel>,
) -> AnyElement {
    let row = &frame.display_rows[idx];
    let bg = if idx.is_multiple_of(2) {
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
pub(super) fn render_pending_row(
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
