//! 单行渲染：行号 + 复选框 + 各单元格；单元格点击分发（下钻 / 标量编辑 / 只读查看）。

use gpui::{
    Context, Hsla, InteractiveElement as _, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, px,
};
use gpui_component::h_flex;

use super::ResultPanel;
use super::cell::Cell;
use super::flatten::Column;
use super::table::{CELL_PREVIEW_MAX, CELL_WIDTH, ROW_HEIGHT, sanitize_inline, truncate};

#[allow(clippy::too_many_arguments)]
pub(super) fn render_row(
    checkbox: gpui::AnyElement,
    row_num_width: gpui::Pixels,
    row_idx_in_view: usize,
    row_idx_in_data: usize,
    cells: &[Cell],
    visible_cols: &[usize],
    columns: &[Column],
    doc: serde_json::Value,
    fg: Hsla,
    muted: Hsla,
    border: Hsla,
    muted_bg: Hsla,
    mono_font: SharedString,
    allow_edit: bool,
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
    // 下钻前导列展示用的行标识：优先 _id，没有就用 id（很多集合用 id 而非 _id）
    let row_ident = doc.get("_id").or_else(|| doc.get("id")).cloned();
    for &ci in visible_cols {
        let cell = &cells[ci];
        let column = &columns[ci];
        // cell.text 原值保留（编辑/查看/导出用），仅清洗显示预览的换行，避免 GPUI 单行断言 panic
        let preview = sanitize_inline(&truncate(&cell.text, CELL_PREVIEW_MAX));
        let is_null = cell.kind == "null" && preview.is_empty();
        // 数字类型列右对齐（与 dbclient is_right 同款）
        let is_right = matches!(column.kind, "int" | "double" | "decimal");
        let mf = mono_font.clone();
        // 捕获列信息 + 单元格值，双击 → 弹单元格 dialog（与 dbclient 单元格编辑器同款交互）
        let path_for_click = column.path.clone();
        let kind_for_click = column.kind;
        let text_for_click = cell.text.clone();
        // 嵌套对象/数组单元格显示的是摘要（{N 字段}/[N 项]），双击取该行该字段原值下钻
        let is_nested = matches!(cell.kind, "object" | "array");
        let nested_for_click = if is_nested {
            doc.get(column.path.as_str()).cloned()
        } else {
            None
        };
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
                    let ident_for_click = row_ident.clone();
                    cx.listener(move |panel, e: &gpui::ClickEvent, window, cx| {
                        if e.click_count() < 2 {
                            return;
                        }
                        // 嵌套对象/数组：下钻查看（对象层进去可编辑，数组层只读），
                        // 传当前行 _id 作回写定位上下文 + 行标识作下钻层前导列
                        if is_nested {
                            if let Some(v) = nested_for_click.clone() {
                                panel.drill_into(
                                    path_for_click.clone(),
                                    id_for_click.clone(),
                                    ident_for_click.clone(),
                                    v,
                                    window,
                                    cx,
                                );
                            }
                            return;
                        }
                        // 标量编辑（仅 allow_edit 视图）：
                        // - 顶层：行 _id + 列名
                        // - 下钻对象层：顶层 _id + 完整 dotted 路径
                        // - 其余（数组层 / 派生只读视图 / 无 _id）：只读查看
                        if allow_edit
                            && panel.can_write()
                            && !panel.is_drilled()
                            && let Some(id) = &id_for_click
                        {
                            panel.open_cell_edit_dialog(
                                id.clone(),
                                path_for_click.clone(),
                                kind_for_click,
                                text_for_click.clone(),
                                window,
                                cx,
                            );
                            return;
                        }
                        if allow_edit
                            && panel.can_write()
                            && panel.drill_editable()
                            && let Some(pid) = panel.drill_parent_id()
                        {
                            panel.open_cell_edit_dialog(
                                pid,
                                panel.drill_full_path(&path_for_click),
                                kind_for_click,
                                text_for_click.clone(),
                                window,
                                cx,
                            );
                            return;
                        }
                        panel.open_cell_dialog(
                            path_for_click.clone(),
                            kind_for_click,
                            text_for_click.clone(),
                            window,
                            cx,
                        );
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
                        } else if is_nested {
                            format!("{preview} ›")
                        } else {
                            preview
                        })),
                ),
        );
    }
    row.into_any_element()
}
