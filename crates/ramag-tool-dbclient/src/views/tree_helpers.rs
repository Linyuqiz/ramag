//! TableTreePanel 的辅助渲染 / 工具函数（从 table_tree.rs 拆出，避免单文件过大）

use gpui::{AnyElement, IntoElement, ParentElement, SharedString, Styled, div, px};
use gpui_component::h_flex;
use ramag_domain::entities::Column;

/// 列子节点：主键 + 列名 + NOT NULL 标记 + raw_type。长名不截断，靠外层横滚；行高 28px 配 uniform_list
pub(super) fn render_column_row(col: &Column, fg: gpui::Hsla, muted_fg: gpui::Hsla) -> AnyElement {
    let pk_label = if col.is_primary_key { "🔑 " } else { "" };
    let null_mark = if col.nullable { "" } else { " *" };
    h_flex()
        .h(px(28.0))
        .flex_none()
        .pl(px(56.0))
        .pr_2()
        .gap_2()
        .items_center()
        .child(
            div()
                .text_xs()
                .text_color(fg)
                .whitespace_nowrap()
                .child(format!("{}{}{}", pk_label, col.name.clone(), null_mark)),
        )
        .child(
            div()
                .text_xs()
                .text_color(muted_fg)
                .whitespace_nowrap()
                .child(col.data_type.raw_type.clone()),
        )
        .into_any_element()
}

/// 加载中 / 错误占位行：缩进同列子节点，单行 ellipsis 截断，行高 28px
pub(super) fn render_columns_placeholder(
    text: impl Into<SharedString>,
    color: gpui::Hsla,
) -> AnyElement {
    div()
        .w_full()
        .h(px(28.0))
        .flex_none()
        .pl(px(56.0))
        .pr_2()
        .pt(px(6.0))
        .text_xs()
        .text_color(color)
        .whitespace_nowrap()
        .overflow_hidden()
        .text_ellipsis()
        .child(text.into())
        .into_any_element()
}
