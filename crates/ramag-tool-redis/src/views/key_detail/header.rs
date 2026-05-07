//! 详情页 header：key 名 + 元信息（DB/类型/TTL/元素数/大小）+ 添加 / 删除按钮

use gpui::{ClickEvent, Context, IntoElement, ParentElement, Styled, div, prelude::*, px};
use gpui_component::{
    IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use ramag_domain::entities::RedisValue;

use super::helpers::format_ttl_ms;
use super::{KeyDetailEvent, KeyDetailPanel};

#[allow(clippy::too_many_arguments)]
pub(super) fn render_header(
    panel: &KeyDetailPanel,
    key: &str,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    accent: gpui::Hsla,
    border: gpui::Hsla,
    cx: &mut Context<KeyDetailPanel>,
) -> impl IntoElement + use<> {
    let ttl_label = match panel.ttl_ms {
        Some(-1) => "永久".to_string(),
        Some(-2) => "已过期".to_string(),
        Some(ms) if ms >= 0 => format_ttl_ms(ms),
        _ => "—".to_string(),
    };
    let key_for_ttl = key.to_string();
    let ttl_ms_for_event = panel.ttl_ms;
    let db = panel.db;
    let value_clone = panel.value.clone();

    // 元信息行（第二行）
    let mut info_row = h_flex()
        .gap(px(10.0))
        .text_xs()
        .text_color(muted_fg)
        .child(div().child(format!("DB {db}")));

    // 类型 chip：色点 + 类型名，颜色与 Key 树徽标一致
    if let Some((label, color)) = value_clone.as_ref().and_then(redis_type_label_color) {
        info_row = info_row.child(
            h_flex()
                .items_center()
                .gap(px(5.0))
                .child(
                    div()
                        .w(px(7.0))
                        .h(px(7.0))
                        .rounded_full()
                        .bg(color)
                        .flex_none(),
                )
                .child(div().child(label.to_string())),
        );
    }

    // TTL 行：accent 颜色 + 可点击 → emit RequestEditTtl
    info_row = info_row.child(
        div()
            .id("ttl-edit-trigger")
            .text_color(accent)
            .cursor_pointer()
            .hover(|this| this.opacity(0.75))
            .child(format!("TTL {ttl_label} ✎"))
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                cx.emit(KeyDetailEvent::RequestEditTtl(
                    key_for_ttl.clone(),
                    ttl_ms_for_event,
                ));
            })),
    );

    if let Some(n) = value_clone.as_ref().and_then(|v| v.len()) {
        info_row = info_row.child(div().child(format!("{n} 元素")));
    }

    info_row = info_row.child(render_size_chip(
        panel.key_size_bytes,
        panel.estimating_size,
        muted_fg,
        accent,
        cx,
    ));

    // 新增按钮（按容器类型）+ 删除 Key 按钮
    let mut header = h_flex()
        .w_full()
        .px(px(14.0))
        .py(px(10.0))
        .border_b_1()
        .border_color(border)
        .gap(px(12.0))
        .items_center()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(4.0))
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(key.to_string()),
                )
                .child(info_row),
        );

    let key_owned = key.to_string();
    if let Some(value) = value_clone.as_ref() {
        let key_for_emit = key_owned.clone();
        header = match value {
            RedisValue::Hash(_) => header.child(add_btn(
                "redis-hash-add-field",
                "新增 Hash 字段",
                cx,
                move || KeyDetailEvent::RequestAddHashField(key_for_emit.clone()),
            )),
            RedisValue::List(_) => header.child(add_btn(
                "redis-list-add-elem",
                "新增 List 元素",
                cx,
                move || KeyDetailEvent::RequestAddListElement(key_for_emit.clone()),
            )),
            RedisValue::Set(_) => header.child(add_btn(
                "redis-set-add-elem",
                "新增 Set 元素",
                cx,
                move || KeyDetailEvent::RequestAddSetElement(key_for_emit.clone()),
            )),
            RedisValue::ZSet(_) => header.child(add_btn(
                "redis-zset-add-elem",
                "新增 ZSet 成员",
                cx,
                move || KeyDetailEvent::RequestAddZSetElement(key_for_emit.clone()),
            )),
            RedisValue::Stream(_) => header.child(add_btn(
                "redis-stream-add-entry",
                "新增 Stream 条目",
                cx,
                move || KeyDetailEvent::RequestAddStreamEntry(key_for_emit.clone()),
            )),
            _ => header,
        };
    }

    let key_for_del = key_owned.clone();
    header.child(
        Button::new("redis-key-delete")
            .danger()
            .small()
            .icon(ramag_ui::icons::trash())
            .tooltip("删除 Key")
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                cx.emit(KeyDetailEvent::RequestDeleteKey(key_for_del.clone()));
            })),
    )
}

fn add_btn<F>(
    id: &'static str,
    tooltip: &'static str,
    cx: &mut Context<KeyDetailPanel>,
    make_event: F,
) -> impl IntoElement + use<F>
where
    F: Fn() -> KeyDetailEvent + 'static,
{
    Button::new(id)
        .outline()
        .small()
        .icon(IconName::Plus)
        .tooltip(tooltip)
        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
            cx.emit(make_event());
        }))
}

/// MEMORY USAGE 显示 chip：未估算时显示 [字节数] 按钮 → 触发 estimate_size；
/// 已估算时显示具体字节数（人类可读单位）
fn render_size_chip(
    bytes: Option<u64>,
    estimating: bool,
    muted_fg: gpui::Hsla,
    accent: gpui::Hsla,
    cx: &mut Context<KeyDetailPanel>,
) -> impl IntoElement + use<> {
    if let Some(n) = bytes {
        let label = format!("{}（{}）", human_readable_bytes(n), n);
        div()
            .id("size-result")
            .text_color(muted_fg)
            .child(format!("📊 {label}"))
            .into_any_element()
    } else if estimating {
        div()
            .id("size-loading")
            .text_color(muted_fg)
            .child("📊 估算中...")
            .into_any_element()
    } else {
        div()
            .id("size-trigger")
            .text_color(accent)
            .cursor_pointer()
            .hover(|this| this.opacity(0.75))
            .child("📊 估算大小")
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.estimate_size(cx)))
            .into_any_element()
    }
}

fn human_readable_bytes(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = n as f64;
    let mut idx = 0;
    while size >= 1024.0 && idx < UNITS.len() - 1 {
        size /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{n} B")
    } else {
        format!("{size:.2} {}", UNITS[idx])
    }
}

/// 由 RedisValue variant 推导（label, 类型色）
/// 与 `key_tree::type_color_solid` / `key_create` 中色板保持一致
pub(super) fn redis_type_label_color(v: &RedisValue) -> Option<(&'static str, gpui::Hsla)> {
    use gpui::hsla;
    match v {
        RedisValue::Text(_) | RedisValue::Bytes(_) => {
            Some(("String", hsla(210.0 / 360.0, 0.6, 0.55, 1.0)))
        }
        RedisValue::List(_) => Some(("List", hsla(140.0 / 360.0, 0.5, 0.5, 1.0))),
        RedisValue::Hash(_) => Some(("Hash", hsla(280.0 / 360.0, 0.55, 0.6, 1.0))),
        RedisValue::Set(_) => Some(("Set", hsla(40.0 / 360.0, 0.85, 0.55, 1.0))),
        RedisValue::ZSet(_) => Some(("ZSet", hsla(20.0 / 360.0, 0.7, 0.55, 1.0))),
        RedisValue::Stream(_) => Some(("Stream", hsla(330.0 / 360.0, 0.55, 0.55, 1.0))),
        _ => None,
    }
}
