//! 仿 Paste.app 大卡片：彩色标题条（类型/时间/来源图标）+ 主体（图片棋盘格/文本）+ 底部元信息。
//! 双击卡片 = 粘贴

use std::sync::Arc;

use chrono::Utc;
use gpui::{
    ClickEvent, Context, Hsla, IntoElement, ParentElement, SharedString, Styled, div, img,
    prelude::*, px,
};
use gpui::{Image, ImageFormat, ImageSource};
use gpui_component::{ActiveTheme, h_flex, v_flex};
use ramag_domain::entities::{ClipItem, ClipKind};

use super::ClipboardDrawer;
use crate::views::helpers::relative_time;

/// 卡片尺寸（仿 Paste 大卡片）
const CARD_W: f32 = 232.0;

impl ClipboardDrawer {
    pub(super) fn render_card(
        &self,
        ix: usize,
        item: &ClipItem,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 临时借用取 owned 颜色，释放 theme 借用（card_body 需 &mut cx 解密图片）
        let border = cx.theme().border;
        let secondary = cx.theme().secondary;
        let muted = cx.theme().muted_foreground;
        let selected = ix == self.selected;
        let header_bg = kind_color(item.kind);
        let blue = gpui::hsla(212.0 / 360.0, 1.0, 0.52, 1.0);
        // 图片缩略图解密（&mut cx）提前算，避免与后续不可变借用冲突
        let thumb = if matches!(item.kind, ClipKind::Image) {
            self.thumb_image(item, cx)
        } else {
            None
        };
        let header = self.card_header(item, header_bg, cx).into_any_element();
        let body = card_body(item, thumb);

        v_flex()
            .id(SharedString::from(format!("drawer-card-{}", item.id)))
            .w(px(CARD_W))
            .h_full()
            .flex_none()
            .rounded(px(10.0))
            .overflow_hidden()
            .border_2()
            .border_color(if selected { blue } else { border })
            .bg(secondary)
            // 单击选中，双击粘贴
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                if ev.click_count() >= 2 {
                    this.paste(ix, window, cx);
                } else {
                    this.selected = ix;
                    cx.notify();
                }
            }))
            .child(header)
            .child(body)
            .child(card_footer(item, muted))
    }

    /// 标题条：左上类型名 + 时间，右上来源应用图标
    fn card_header(&self, item: &ClipItem, bg: Hsla, cx: &Context<Self>) -> impl IntoElement {
        let mut sub = gpui::white();
        sub.a = 0.75;
        let icon = self.source_icon(item, cx);

        h_flex()
            .w_full()
            .flex_none()
            .h(px(56.0))
            .px(px(12.0))
            .py(px(8.0))
            .bg(bg)
            .items_start()
            .justify_between()
            .child(
                v_flex()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(gpui::white())
                            .child(item.kind.label_en()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(sub)
                            .child(relative_time(item.last_used_at, Utc::now())),
                    ),
            )
            .children(icon)
    }

    /// 来源应用图标（内存 PNG → gpui Image，按 bundle_id 缓存）
    fn source_icon(&self, item: &ClipItem, _cx: &Context<Self>) -> Option<gpui::AnyElement> {
        let bundle = item.source.as_ref().map(|s| s.bundle_id.as_str())?;
        let png = self.service().app_icon(bundle)?;
        let image = Arc::new(Image::from_bytes(ImageFormat::Png, png.as_ref().clone()));
        Some(
            img(ImageSource::Image(image))
                .size(px(34.0))
                .rounded(px(7.0))
                .into_any_element(),
        )
    }
}

/// 主体：图片显示棋盘格透明底 + 缩略图（解密内存图片）；文本/其它显示内容
fn card_body(item: &ClipItem, thumb: Option<Arc<Image>>) -> gpui::AnyElement {
    match item.kind {
        ClipKind::Image => div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            // 棋盘格透明背景层（svg pattern 平铺）
            .child(
                gpui::svg()
                    .absolute()
                    .inset_0()
                    .size_full()
                    .path("icons/checker.svg"),
            )
            .when_some(thumb, |this, image| {
                this.child(
                    img(image)
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(gpui::ObjectFit::Contain),
                )
            })
            .into_any_element(),
        _ => div()
            .flex_1()
            .min_h_0()
            .w_full()
            .p(px(12.0))
            .text_sm()
            .overflow_hidden()
            .child(item.preview.clone())
            .into_any_element(),
    }
}

/// 底部元信息：图片报尺寸，文本报字符数
fn card_footer(item: &ClipItem, muted: Hsla) -> impl IntoElement {
    let label = match item.kind {
        ClipKind::Image => item
            .image_dims
            .map(|(w, h)| format!("{w} × {h}"))
            .unwrap_or_default(),
        ClipKind::Files => format!("{} 个文件", item.files.len()),
        _ => {
            let n = item.text.as_deref().map(|t| t.chars().count()).unwrap_or(0);
            format!("{n} characters")
        }
    };
    div()
        .w_full()
        .flex_none()
        .h(px(32.0))
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .text_color(muted)
        .child(label)
}

/// 标题条配色（Image 绿 / Text 深 / Link 蓝 / Color 紫 / Files 橙），对齐 Paste 观感
fn kind_color(kind: ClipKind) -> Hsla {
    use gpui::hsla;
    match kind {
        ClipKind::Image => hsla(145.0 / 360.0, 0.62, 0.45, 1.0),
        ClipKind::Text => hsla(0.0, 0.0, 0.17, 1.0),
        ClipKind::Link => hsla(212.0 / 360.0, 0.7, 0.5, 1.0),
        ClipKind::Color => hsla(280.0 / 360.0, 0.5, 0.5, 1.0),
        ClipKind::Files => hsla(32.0 / 360.0, 0.8, 0.5, 1.0),
    }
}
