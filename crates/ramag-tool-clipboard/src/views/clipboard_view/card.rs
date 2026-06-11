//! 单张剪贴卡片渲染：类型角标 + 预览 + 来源 + 时间 + 钉住/复制/删除按钮

use chrono::Utc;
use gpui::{
    ClickEvent, Context, Hsla, IntoElement, ParentElement, SharedString, Styled, div, img,
    prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Icon, Sizable as _, button::Button, button::ButtonVariants as _, h_flex, v_flex,
};
use ramag_domain::entities::{ClipItem, ClipKind, parse_hex_color};
use ramag_ui::icons;

use super::ClipboardView;
use crate::views::helpers::relative_time;

impl ClipboardView {
    pub(super) fn render_card(&self, item: &ClipItem, cx: &mut Context<Self>) -> impl IntoElement {
        // 临时借用取 owned 颜色，释放 theme 借用，否则与 card_preview 的 &mut cx 冲突
        let accent = cx.theme().accent;
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let row_hover = cx.theme().secondary;
        let kind_badge = {
            let theme = cx.theme();
            kind_badge(item.kind, theme).into_any_element()
        };

        let is_selected = self.selected.as_ref() == Some(&item.id);
        let id = item.id.clone();
        let row_id = SharedString::from(format!("clip-card-{}", item.id));

        let preview = self.card_preview(item, cx);
        let source = item
            .source
            .as_ref()
            .map(|s| s.name.clone())
            .unwrap_or_default();
        let time = relative_time(item.last_used_at, Utc::now());

        let item_copy = item.clone();
        let item_dbl = item.clone();
        let item_del = item.clone();

        let mut card = v_flex()
            .id(row_id)
            .w_full()
            .h(px(76.0))
            .flex_none()
            .px(px(12.0))
            .py(px(8.0))
            .gap(px(4.0))
            .border_b_1()
            .border_color(border)
            .cursor_pointer()
            // 单击选中，双击复制回剪贴板
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                if ev.click_count() >= 2 {
                    this.copy_clip(item_dbl.clone(), cx);
                } else {
                    this.select_id(id.clone(), cx);
                }
            }))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(kind_badge)
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(muted).child(time)),
            )
            .child(
                div()
                    .w_full()
                    .text_sm()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(preview),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(muted)
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(source),
                    )
                    .child(card_action_btn(
                        "copy",
                        icons::copy(),
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.copy_clip(item_copy.clone(), cx);
                        }),
                    ))
                    .child(card_action_btn(
                        "del",
                        icons::trash(),
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.delete_clip(item_del.clone(), cx);
                        }),
                    )),
            );

        if is_selected {
            let mut active = accent;
            active.a = 0.14;
            card = card.bg(active);
        } else {
            card = card.hover(move |this| this.bg(row_hover));
        }
        card
    }

    /// 卡片中部预览：颜色显示色卡，图片显示缩略图，其余显示文本
    fn card_preview(&self, item: &ClipItem, cx: &mut Context<Self>) -> gpui::AnyElement {
        match item.kind {
            ClipKind::Color => {
                let swatch = item
                    .text
                    .as_deref()
                    .and_then(parse_hex_color)
                    .map(|(r, g, b)| {
                        Hsla::from(gpui::rgb(
                            (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b),
                        ))
                    });
                h_flex()
                    .items_center()
                    .gap(px(8.0))
                    .when_some(swatch, |this, color| {
                        this.child(div().size(px(16.0)).rounded(px(3.0)).bg(color))
                    })
                    .child(item.preview.clone())
                    .into_any_element()
            }
            ClipKind::Image => match self.image_for(item, true, cx) {
                Some(image) => img(image).max_h(px(28.0)).into_any_element(),
                None => div().child(item.preview.clone()).into_any_element(),
            },
            _ => div().child(item.preview.clone()).into_any_element(),
        }
    }
}

/// 类型角标（彩色小标签）
fn kind_badge(kind: ClipKind, theme: &gpui_component::Theme) -> impl IntoElement {
    let bg = kind_color(kind, theme);
    div()
        .px(px(5.0))
        .py(px(1.0))
        .rounded(px(3.0))
        .text_xs()
        .bg(bg)
        .text_color(theme.background)
        .child(kind.label())
}

fn kind_color(kind: ClipKind, theme: &gpui_component::Theme) -> Hsla {
    use gpui::hsla;
    match kind {
        ClipKind::Text => theme.muted_foreground,
        ClipKind::Link => hsla(210.0 / 360.0, 0.65, 0.55, 1.0),
        ClipKind::Color => hsla(280.0 / 360.0, 0.55, 0.55, 1.0),
        ClipKind::Image => hsla(140.0 / 360.0, 0.55, 0.45, 1.0),
        ClipKind::Files => hsla(35.0 / 360.0, 0.8, 0.5, 1.0),
    }
}

fn card_action_btn(
    id: &'static str,
    icon: Icon,
    on_click: impl Fn(&ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    Button::new(id)
        .ghost()
        .xsmall()
        .icon(icon.size_3())
        .on_click(on_click)
}
