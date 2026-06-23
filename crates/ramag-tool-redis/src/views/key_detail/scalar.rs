//! 标量值（String / Bytes）渲染：视图模式切换（Raw/JSON/Hex/base64，按内容自动选默认）
//! + Gzip 提示 + 内容区（双击编辑，仅 Text）

use gpui::{ClickEvent, Context, IntoElement, ParentElement, Styled, Window, div, prelude::*, px};
use gpui_component::{
    Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use ramag_domain::entities::RedisValue;

use super::{KeyDetailEvent, KeyDetailPanel};
use crate::views::value_display::{self, ViewMode};

#[allow(clippy::too_many_arguments)]
pub(super) fn render_scalar(
    key: &str,
    v: &RedisValue,
    view_mode: Option<ViewMode>,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    border: gpui::Hsla,
    cx: &mut Context<KeyDetailPanel>,
    _window: &Window,
) -> impl IntoElement + use<> {
    // 取原始字节流（用于 Gzip 检测 + 渲染）
    let raw_bytes: Vec<u8> = match v {
        RedisValue::Text(s) => s.as_bytes().to_vec(),
        RedisValue::Bytes(b) => b.clone(),
        _ => Vec::new(),
    };

    // 自动 Gzip 解压（成功则用解压结果替代原 bytes 渲染）
    let (display_bytes, gzip_hint) = match value_display::try_decompress_gzip(&raw_bytes) {
        Some(decoded) => {
            let hint = format!(
                "🗜️ 检测到 Gzip 压缩，已自动解压（原 {} bytes → {} bytes）",
                raw_bytes.len(),
                decoded.len()
            );
            (decoded, Some(hint))
        }
        None => (raw_bytes.clone(), None),
    };

    // 视图模式：用户未手动选则按内容自动判定（JSON 美化 / Raw）
    let mode = view_mode.unwrap_or_else(|| value_display::auto_view_mode(&display_bytes));
    let content_text = match v {
        RedisValue::Text(_) => match std::str::from_utf8(&display_bytes) {
            Ok(s) => value_display::render_text(s, mode),
            Err(_) => value_display::render_bytes(&display_bytes, mode),
        },
        _ => value_display::render_bytes(&display_bytes, mode),
    };

    // 编辑入口仅对 Text 类型开放（Bytes 二进制不支持文本编辑）：双击内容区打开编辑窗口
    let edit_target: Option<(String, String)> = match v {
        RedisValue::Text(s) => Some((key.to_string(), s.clone())),
        _ => None,
    };

    let content_div = div()
        .id("redis-scalar-content")
        .flex_1()
        .min_w_0()
        .p(px(10.0))
        .border_1()
        .border_color(border)
        .rounded(px(4.0))
        .text_sm()
        .text_color(fg)
        .font_family("monospace")
        .when_some(edit_target, |this, (k, s)| {
            this.cursor_pointer()
                .on_click(cx.listener(move |_, e: &ClickEvent, _, cx| {
                    if e.click_count() >= 2 {
                        cx.emit(KeyDetailEvent::RequestEditValue(k.clone(), s.clone()));
                    }
                }))
        })
        .child(content_text);

    let content_row = h_flex().w_full().child(content_div);

    // 视图模式切换：Raw / JSON / Hex / base64，高亮当前生效模式；点击即固定为手动模式
    let mode_row = h_flex().gap(px(4.0)).children(
        [
            (ViewMode::Raw, "Raw"),
            (ViewMode::Json, "JSON"),
            (ViewMode::Hex, "Hex"),
            (ViewMode::Base64, "base64"),
        ]
        .into_iter()
        .map(|(m, label)| {
            Button::new(label)
                .xsmall()
                .ghost()
                .selected(m == mode)
                .label(label)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_value_view_mode(m, cx);
                }))
        }),
    );

    v_flex()
        .w_full()
        .gap(px(8.0))
        .child(mode_row)
        // Gzip 自动解压提示
        .when_some(gzip_hint, |this, hint| {
            this.child(
                div()
                    .px(px(10.0))
                    .py(px(6.0))
                    .text_xs()
                    .text_color(muted_fg)
                    .border_1()
                    .border_color(border)
                    .rounded(px(4.0))
                    .child(hint),
            )
        })
        .child(content_row)
}
