//! Set 块：每行带删除按钮

use gpui::{ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div, px};
use gpui_component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use ramag_domain::entities::RedisValue;

use super::{KeyDetailEvent, KeyDetailPanel};

#[allow(clippy::too_many_arguments)]
pub(super) fn render_set_block(
    panel: &mut Context<KeyDetailPanel>,
    key: String,
    items: &[RedisValue],
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
    for (i, item) in items.iter().enumerate() {
        let preview = item.display_preview(256);
        let raw_member = match item {
            RedisValue::Text(s) => s.clone(),
            other => other.display_preview(8192),
        };
        let key_for_emit = key.clone();
        let del_id = SharedString::from(format!("set-del-{i}"));
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
                        .w(px(40.0))
                        .text_xs()
                        .text_color(muted_fg)
                        .flex_none()
                        .child(format!("{i}")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(fg)
                        .font_family("monospace")
                        .child(preview),
                )
                .child(
                    Button::new(del_id)
                        .ghost()
                        .small()
                        .icon(ramag_ui::icons::trash())
                        .tooltip("删除该成员")
                        .on_click(panel.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(KeyDetailEvent::RequestDeleteSetElement(
                                key_for_emit.clone(),
                                raw_member.clone(),
                            ));
                        })),
                ),
        );
    }
    rows
}
