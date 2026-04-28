//! TableTreePanel 的辅助渲染 / 工具函数（从 table_tree.rs 拆出，避免单文件过大）

use gpui::{AnyElement, IntoElement, ParentElement, SharedString, Styled, div, px};
use gpui_component::h_flex;
use ramag_domain::entities::Column;

/// 整数加千位分隔符："1234567" → "1,234,567"
pub(super) fn format_thousands(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

/// 渲染单个列结构子节点：🔑 主键 + 列名 + * NOT NULL + raw_type
/// 长列名 / 长类型不截断，依赖外层横向滚动容器查看
pub(super) fn render_column_row(
    col: &Column,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
) -> AnyElement {
    let pk_label = if col.is_primary_key { "🔑 " } else { "" };
    let null_mark = if col.nullable { "" } else { " *" };
    h_flex()
        .pl(px(56.0))
        .pr_2()
        .py(px(2.0))
        .gap_2()
        .items_center()
        .child(
            div()
                .text_xs()
                .text_color(fg)
                .whitespace_nowrap()
                .child(format!(
                    "{}{}{}",
                    pk_label,
                    col.name.clone(),
                    null_mark
                )),
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

/// 加载中 / 错误的占位行（缩进与列子节点一致）
///
/// 渲染策略：单行 + 超长 ellipsis 截断（与列名行一致）
/// - 索引 / 外键的描述行可能很长（多列复合索引），不限制就会折行破坏视觉一致
/// - 用户要看完整内容拖宽侧栏即可
pub(super) fn render_columns_placeholder(
    text: impl Into<SharedString>,
    color: gpui::Hsla,
) -> AnyElement {
    div()
        .w_full()
        .pl(px(56.0))
        .pr_2()
        .py_1()
        .text_xs()
        .text_color(color)
        .whitespace_nowrap()
        .overflow_hidden()
        .text_ellipsis()
        .child(text.into())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::format_thousands;

    #[test]
    fn thousands() {
        assert_eq!(format_thousands(0), "0");
        assert_eq!(format_thousands(123), "123");
        assert_eq!(format_thousands(1234), "1,234");
        assert_eq!(format_thousands(1234567), "1,234,567");
        assert_eq!(format_thousands(1_000_000_000), "1,000,000,000");
    }
}
