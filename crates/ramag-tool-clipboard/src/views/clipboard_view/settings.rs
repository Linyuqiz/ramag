//! 设置面板：采集开关 / 图片采集 / 自动粘贴 / 条数上限 / 保留天数 / 清空

use gpui::{ClickEvent, Context, IntoElement, ParentElement, Styled, Window, div, prelude::*, px};
use gpui_component::{
    ActiveTheme, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    switch::Switch,
    v_flex,
};
use ramag_ui::open_confirm;

use super::ClipboardView;

impl ClipboardView {
    pub(super) fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let border = theme.border;
        let s = self.settings.clone();

        v_flex()
            .id("clip-settings-scroll")
            .size_full()
            .p(px(16.0))
            .gap(px(14.0))
            .overflow_y_scroll()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("剪贴板设置"),
            )
            .child(self.toggle_row(
                "clip-enabled",
                "启用采集",
                "关闭后停止记录新内容",
                s.enabled,
                muted,
                cx.listener(|this, _: &bool, _, cx| {
                    let mut next = this.settings.clone();
                    next.enabled = !next.enabled;
                    this.save_settings(next, cx);
                }),
            ))
            .child(self.toggle_row(
                "clip-images",
                "采集图片",
                "记录复制的图片（占用磁盘较多）",
                s.capture_images,
                muted,
                cx.listener(|this, _: &bool, _, cx| {
                    let mut next = this.settings.clone();
                    next.capture_images = !next.capture_images;
                    this.save_settings(next, cx);
                }),
            ))
            .child(self.toggle_row(
                "clip-autopaste",
                "自动粘贴",
                "抽屉选中后自动粘贴到当前应用（需辅助功能权限）",
                s.auto_paste,
                muted,
                cx.listener(|this, _: &bool, _, cx| {
                    let mut next = this.settings.clone();
                    next.auto_paste = !next.auto_paste;
                    this.save_settings(next, cx);
                }),
            ))
            .child(self.stepper_row(
                "count",
                "条数上限",
                &format!("{} 条", s.max_items),
                // 数量级步长：小值小步、大值大步，几下即可调到 50 万
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    let mut next = this.settings.clone();
                    let step = count_step(next.max_items.saturating_sub(1));
                    next.max_items = next.max_items.saturating_sub(step).max(100);
                    this.save_settings(next, cx);
                }),
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    let mut next = this.settings.clone();
                    next.max_items = (next.max_items + count_step(next.max_items)).min(500_000);
                    this.save_settings(next, cx);
                }),
            ))
            .child(self.stepper_row(
                "age",
                "保留天数",
                &format!("{} 天", s.max_age_days),
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    let mut next = this.settings.clone();
                    next.max_age_days = next.max_age_days.saturating_sub(7).max(1);
                    this.save_settings(next, cx);
                }),
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    let mut next = this.settings.clone();
                    next.max_age_days = (next.max_age_days + 7).min(3650);
                    this.save_settings(next, cx);
                }),
            ))
            .child(div().h(px(1.0)).bg(border))
            // 清空历史（移入设置，避免顶栏误触）
            .child(
                h_flex().w_full().items_center().justify_between().child(
                    Button::new("clip-clear-all")
                        .danger()
                        .small()
                        .label("清空历史")
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.confirm_clear(window, cx);
                        })),
                ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("历史全本地存储并加密，不会上传任何服务器"),
            )
    }

    fn toggle_row(
        &self,
        id: &'static str,
        title: &str,
        desc: &str,
        checked: bool,
        muted: gpui::Hsla,
        on_click: impl Fn(&bool, &mut Window, &mut gpui::App) + 'static,
    ) -> impl IntoElement {
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .child(
                v_flex()
                    .gap(px(2.0))
                    .child(div().text_sm().child(title.to_string()))
                    .child(div().text_xs().text_color(muted).child(desc.to_string())),
            )
            .child(Switch::new(id).checked(checked).on_click(on_click))
    }

    fn stepper_row(
        &self,
        id: &str,
        title: &str,
        value: &str,
        on_dec: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
        on_inc: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
    ) -> impl IntoElement {
        // 两个 stepper 的按钮 id 必须唯一，否则 gpui 交互冲突导致点击无响应
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .child(div().text_sm().child(title.to_string()))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        Button::new(format!("{id}-dec"))
                            .ghost()
                            .xsmall()
                            .label("−")
                            .on_click(on_dec),
                    )
                    .child(
                        div()
                            .min_w(px(56.0))
                            .text_sm()
                            .text_center()
                            .child(value.to_string()),
                    )
                    .child(
                        Button::new(format!("{id}-inc"))
                            .ghost()
                            .xsmall()
                            .label("+")
                            .on_click(on_inc),
                    ),
            )
    }

    /// 清空历史二次确认（复用 ramag-ui 通用确认弹窗）
    pub(super) fn confirm_clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entity = cx.entity().clone();
        open_confirm(
            "清空剪贴历史",
            "将删除全部未固定的历史条目，固定项保留。此操作不可撤销。",
            "清空",
            true,
            move |_window, cx| {
                entity.update(cx, |this, cx| this.clear_all(cx));
            },
            window,
            cx,
        );
    }
}

/// 条数上限的数量级步长：当前值越大步进越大，几下即可从两千调到五十万
fn count_step(v: u32) -> u32 {
    match v {
        0..=4_999 => 1_000,
        5_000..=49_999 => 5_000,
        50_000..=199_999 => 50_000,
        _ => 100_000,
    }
}
