//! Hash 块：每行编辑 / 删除按钮

use gpui::{ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div, px};
use gpui_component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use ramag_domain::entities::RedisValue;

use super::{KeyDetailEvent, KeyDetailPanel};

#[allow(clippy::too_many_arguments)]
pub(super) fn render_hash_block(
    panel: &mut Context<KeyDetailPanel>,
    key: String,
    pairs: &[(String, RedisValue)],
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    _accent: gpui::Hsla,
    border: gpui::Hsla,
) -> impl IntoElement + use<> {
    let mut rows = v_flex()
        .w_full()
        .gap(px(0.0))
        .border_1()
        .border_color(border)
        .rounded(px(4.0));
    for (idx, (f, v)) in pairs.iter().enumerate() {
        let field_name = f.clone();
        let value_preview = v.display_preview(256);
        // 编辑用的"原始文本"取最完整可读形态；二进制 Bytes 走 hex 预览
        let value_for_edit = match v {
            RedisValue::Text(s) => s.clone(),
            other => other.display_preview(8192),
        };

        let key_for_edit = key.clone();
        let field_for_edit = field_name.clone();
        let value_for_edit_clone = value_for_edit.clone();
        let key_for_del = key.clone();
        let field_for_del = field_name.clone();

        let edit_id = SharedString::from(format!("hash-edit-{idx}"));
        let del_id = SharedString::from(format!("hash-del-{idx}"));

        rows = rows.child(
            h_flex()
                .w_full()
                .px(px(8.0))
                .py(px(6.0))
                .border_b_1()
                .border_color(border)
                .gap(px(8.0))
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
                        .child(value_preview),
                )
                .child(
                    h_flex()
                        .gap(px(4.0))
                        .flex_none()
                        .child(
                            Button::new(edit_id)
                                .ghost()
                                .small()
                                .icon(ramag_ui::icons::pencil())
                                .tooltip("编辑该字段")
                                .on_click(panel.listener(move |_, _: &ClickEvent, _, cx| {
                                    cx.emit(KeyDetailEvent::RequestEditHashField(
                                        key_for_edit.clone(),
                                        field_for_edit.clone(),
                                        value_for_edit_clone.clone(),
                                    ));
                                })),
                        )
                        .child(
                            Button::new(del_id)
                                .ghost()
                                .small()
                                .icon(ramag_ui::icons::trash())
                                .tooltip("删除该字段")
                                .on_click(panel.listener(move |_, _: &ClickEvent, _, cx| {
                                    cx.emit(KeyDetailEvent::RequestDeleteHashField(
                                        key_for_del.clone(),
                                        field_for_del.clone(),
                                    ));
                                })),
                        ),
                ),
        );
    }
    rows
}
