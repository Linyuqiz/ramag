//! 结果区顶部工具栏：过滤列 / 过滤行 / 导出 / 复制全部。
//! 行数 / 耗时摘要已下沉到底部 status bar（见 mod.rs render_status_bar），与 dbclient 一致

use gpui::{Context, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{
    ActiveTheme, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
};

use super::ResultPanel;

pub(super) fn render(panel: &mut ResultPanel, cx: &mut Context<ResultPanel>) -> impl IntoElement {
    let secondary = cx.theme().secondary;

    h_flex()
        .w_full()
        .flex_none()
        .px(px(8.0))
        .py(px(6.0))
        .gap(px(8.0))
        .items_center()
        .bg(secondary)
        .child(
            div().flex_1().min_w_0().child(
                Input::new(&panel.column_filter)
                    .small()
                    .bordered(false)
                    .focus_bordered(false)
                    .cleanable(true),
            ),
        )
        .child(
            div().flex_1().min_w_0().child(
                Input::new(&panel.row_filter)
                    .small()
                    .bordered(false)
                    .focus_bordered(false)
                    .cleanable(true),
            ),
        )
        .child(
            Button::new("mongo-export")
                .ghost()
                .xsmall()
                .icon(IconName::Inbox)
                .tooltip("导出（待实现）")
                .disabled(true),
        )
        .child(
            Button::new("mongo-copy-all")
                .ghost()
                .xsmall()
                .icon(IconName::Copy)
                .tooltip("复制全部为 JSON")
                .on_click(cx.listener(|panel, _, _, cx| {
                    if let Some(r) = &panel.result {
                        let json = serde_json::to_string_pretty(&r.documents).unwrap_or_default();
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(json));
                    }
                })),
        )
}
