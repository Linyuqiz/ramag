//! Hash 块：uniform_list 行级虚拟化（等高行），双击行编辑字段 + 删除按钮

use std::ops::Range;

use gpui::{
    ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, UniformListScrollHandle,
    div, prelude::*, px, uniform_list,
};
use gpui_component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};
use ramag_domain::entities::RedisValue;

use super::{KeyDetailEvent, KeyDetailPanel};

/// 行高固定 32px：uniform_list 行级虚拟化要求等高
const ROW_H: f32 = 32.0;

pub(super) fn render_hash_block(
    panel: &mut Context<KeyDetailPanel>,
    key: String,
    count: usize,
    scroll: &UniformListScrollHandle,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    border: gpui::Hsla,
) -> impl IntoElement + use<> {
    div()
        .flex_1()
        .min_h_0()
        .border_1()
        .border_color(border)
        .rounded(px(4.0))
        .child(
            uniform_list(
                "hash-rows",
                count,
                panel.processor(move |this, range: Range<usize>, _w, cx| {
                    let Some(RedisValue::Hash(pairs)) = &this.value else {
                        return Vec::new();
                    };
                    range
                        .filter_map(|idx| {
                            let (f, v) = pairs.get(idx)?;
                            Some(
                                hash_row(&key, idx, f, v, fg, muted_fg, border, cx)
                                    .into_any_element(),
                            )
                        })
                        .collect()
                }),
            )
            .track_scroll(scroll)
            .flex_1(),
        )
}

#[allow(clippy::too_many_arguments)]
fn hash_row(
    key: &str,
    idx: usize,
    field: &str,
    value: &RedisValue,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    border: gpui::Hsla,
    cx: &mut Context<KeyDetailPanel>,
) -> impl IntoElement + use<> {
    let field_name = field.to_string();
    let value_preview = value.display_preview(256);
    // 编辑用的"原始文本"取最完整可读形态；二进制 Bytes 走 hex 预览
    let value_for_edit = match value {
        RedisValue::Text(s) => s.clone(),
        other => other.display_preview(8192),
    };
    let key_for_edit = key.to_string();
    let field_for_edit = field_name.clone();
    let value_for_edit_clone = value_for_edit.clone();
    let key_for_del = key.to_string();
    let field_for_del = field_name.clone();
    let row_id = SharedString::from(format!("hash-row-{idx}"));
    let del_id = SharedString::from(format!("hash-del-{idx}"));

    h_flex()
        .id(row_id)
        .h(px(ROW_H))
        .flex_none()
        .w_full()
        .px(px(8.0))
        .border_b_1()
        .border_color(border)
        .gap(px(8.0))
        .items_center()
        .cursor_pointer()
        // 双击该行打开编辑窗口
        .on_click(cx.listener(move |_, e: &ClickEvent, _, cx| {
            if e.click_count() >= 2 {
                cx.emit(KeyDetailEvent::RequestEditHashField(
                    key_for_edit.clone(),
                    field_for_edit.clone(),
                    value_for_edit_clone.clone(),
                ));
            }
        }))
        .child(
            div()
                .w(px(160.0))
                .text_xs()
                .text_color(muted_fg)
                .flex_none()
                .overflow_hidden()
                .text_ellipsis()
                .child(field_name),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(fg)
                .font_family("monospace")
                .overflow_hidden()
                .text_ellipsis()
                .child(value_preview),
        )
        .child(
            Button::new(del_id)
                .ghost()
                .small()
                .icon(ramag_ui::icons::trash())
                .tooltip("删除该字段")
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    cx.emit(KeyDetailEvent::RequestDeleteHashField(
                        key_for_del.clone(),
                        field_for_del.clone(),
                    ));
                })),
        )
}
