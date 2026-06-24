//! 通用渲染辅助：值分发 + 标量小块 + 时长格式化 + 并发 helper

use gpui::{
    AnyElement, Context, IntoElement, ParentElement, Styled, UniformListScrollHandle, div, px,
};
use gpui_component::{h_flex, v_flex};
use ramag_domain::entities::RedisValue;

use super::KeyDetailPanel;
use super::hash_block::render_hash_block;
use super::list_block::render_list_block;
use super::set_block::render_set_block;
use super::stream_block::render_stream_block;
use super::zset_block::render_zset_block;

/// 按 RedisValue variant 分发：容器走带 cx 的方法版（emit 编辑 / 删除）；标量走只读 free fn；
/// String / Bytes 由 mod.rs Render 单独走 `scalar::render_scalar`
#[allow(clippy::too_many_arguments)]
pub(super) fn render_value(
    v: &RedisValue,
    key: &str,
    cx: &mut Context<KeyDetailPanel>,
    scroll: &UniformListScrollHandle,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    border: gpui::Hsla,
) -> AnyElement {
    match v {
        RedisValue::Nil => simple_label("(nil)", muted_fg).into_any_element(),
        RedisValue::Text(s) => string_block(s, fg, border).into_any_element(),
        RedisValue::Bytes(b) => bytes_block(b, fg, muted_fg, border).into_any_element(),
        RedisValue::Int(i) => simple_label(&format!("{i} (integer)"), fg).into_any_element(),
        RedisValue::Float(f) => simple_label(&format!("{f} (double)"), fg).into_any_element(),
        RedisValue::Bool(b) => simple_label(&format!("{b} (bool)"), fg).into_any_element(),
        RedisValue::List(items) => render_list_block(
            cx,
            key.to_string(),
            items.len(),
            scroll,
            fg,
            muted_fg,
            border,
        )
        .into_any_element(),
        RedisValue::Hash(pairs) => render_hash_block(
            cx,
            key.to_string(),
            pairs.len(),
            scroll,
            fg,
            muted_fg,
            border,
        )
        .into_any_element(),
        RedisValue::Set(items) => render_set_block(
            cx,
            key.to_string(),
            items.len(),
            scroll,
            fg,
            muted_fg,
            border,
        )
        .into_any_element(),
        RedisValue::ZSet(pairs) => render_zset_block(
            cx,
            key.to_string(),
            pairs.len(),
            scroll,
            fg,
            muted_fg,
            border,
        )
        .into_any_element(),
        RedisValue::Stream(entries) => {
            render_stream_block(cx, key.to_string(), entries, scroll, fg, muted_fg, border)
                .into_any_element()
        }
        // Array 兜底（命令应答的复合返回，不直接来自 key value）
        RedisValue::Array(items) => array_block(items, fg, muted_fg, border).into_any_element(),
    }
}

pub(super) fn simple_label(s: &str, color: gpui::Hsla) -> impl IntoElement {
    div()
        .p(px(8.0))
        .text_sm()
        .text_color(color)
        .child(s.to_string())
}

pub(super) fn string_block(s: &str, fg: gpui::Hsla, border: gpui::Hsla) -> impl IntoElement {
    div()
        .w_full()
        .p(px(10.0))
        .border_1()
        .border_color(border)
        .rounded(px(4.0))
        .text_sm()
        .text_color(fg)
        .font_family("monospace")
        .child(s.to_string())
}

pub(super) fn bytes_block(
    b: &[u8],
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    border: gpui::Hsla,
) -> impl IntoElement {
    let preview = b
        .iter()
        .take(64)
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    let suffix = if b.len() > 64 { " ..." } else { "" };
    v_flex()
        .gap(px(6.0))
        .child(
            div()
                .text_xs()
                .text_color(muted_fg)
                .child(format!("[{} bytes]", b.len())),
        )
        .child(
            div()
                .w_full()
                .p(px(10.0))
                .border_1()
                .border_color(border)
                .rounded(px(4.0))
                .text_xs()
                .text_color(fg)
                .font_family("monospace")
                .child(format!("{preview}{suffix}")),
        )
}

/// Array 类型的只读列表渲染（命令应答兜底，不带行操作按钮）
fn array_block(
    items: &[RedisValue],
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    border: gpui::Hsla,
) -> impl IntoElement {
    let mut rows = v_flex()
        .w_full()
        .gap(px(0.0))
        .border_1()
        .border_color(border)
        .rounded(px(4.0));
    for (i, item) in items.iter().enumerate() {
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
                        .child(item.display_preview(256)),
                ),
        );
    }
    rows
}

/// 把毫秒数格式化为人类可读
pub(super) fn format_ttl_ms(ms: i64) -> String {
    let secs = ms / 1000;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else if secs < 86_400 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}d {}h", secs / 86_400, (secs % 86_400) / 3600)
    }
}

/// 简单的并发 await 两个 future（不引入额外依赖）
/// 借 GPUI 已有的 futures crate（workspace 默认包含）
pub(super) async fn futures_join<A, B, RA, RB>(a: A, b: B) -> (RA, RB)
where
    A: Future<Output = RA>,
    B: Future<Output = RB>,
{
    use futures::future::join;
    join(a, b).await
}
