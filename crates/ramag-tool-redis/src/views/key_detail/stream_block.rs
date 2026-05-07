//! Stream 块：每条 entry 显示 ID + 字段对 + 删除按钮

use gpui::{ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div, px};
use gpui_component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use ramag_domain::entities::StreamEntry;

use super::{KeyDetailEvent, KeyDetailPanel};

#[allow(clippy::too_many_arguments)]
pub(super) fn render_stream_block(
    panel: &mut Context<KeyDetailPanel>,
    key: String,
    entries: &[StreamEntry],
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    _accent: gpui::Hsla,
    border: gpui::Hsla,
) -> impl IntoElement + use<> {
    let mut blocks = v_flex().w_full().gap(px(8.0));
    for (idx, e) in entries.iter().enumerate() {
        let mut fields = v_flex().w_full().gap(px(2.0)).pl(px(12.0));
        for (k, v) in &e.fields {
            fields = fields.child(
                h_flex()
                    .w_full()
                    .gap(px(8.0))
                    .child(
                        div()
                            .w(px(140.0))
                            .text_xs()
                            .text_color(muted_fg)
                            .flex_none()
                            .child(k.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(fg)
                            .font_family("monospace")
                            .child(v.clone()),
                    ),
            );
        }
        let entry_id = e.id.clone();
        let id_for_del = entry_id.clone();
        let key_for_del = key.clone();
        let del_btn_id = SharedString::from(format!("stream-del-{idx}"));
        blocks = blocks.child(
            v_flex()
                .w_full()
                .p(px(8.0))
                .border_1()
                .border_color(border)
                .rounded(px(4.0))
                .gap(px(4.0))
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(fg)
                                .child(entry_id),
                        )
                        .child(
                            Button::new(del_btn_id)
                                .ghost()
                                .small()
                                .icon(ramag_ui::icons::trash())
                                .tooltip("删除该条目")
                                .on_click(panel.listener(move |_, _: &ClickEvent, _, cx| {
                                    cx.emit(KeyDetailEvent::RequestDeleteStreamEntry(
                                        key_for_del.clone(),
                                        id_for_del.clone(),
                                    ));
                                })),
                        ),
                )
                .child(fields),
        );
    }
    blocks
}
