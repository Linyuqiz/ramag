//! ClipboardView Render：顶部工具条（搜索 + 类型筛选 + 设置）+ 左卡片流 + 右详情

use gpui::{
    ClickEvent, Context, IntoElement, ParentElement, Render, SharedString, Styled, Window, div,
    prelude::*, px, uniform_list,
};
use gpui_component::{
    ActiveTheme, Selectable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    v_flex,
};
use ramag_domain::entities::ClipKind;
use ramag_ui::icons;

use super::ClipboardView;
use crate::actions::{
    CopySelectedClip, DeleteSelectedClip, FocusClipSearch, SelectNextClip, SelectPrevClip,
};

impl Render for ClipboardView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(n) = self.pending_notification.take() {
            window.push_notification(n, cx);
        }

        let theme = cx.theme();
        let border = theme.border;
        let muted = theme.muted_foreground;
        let visible = self.visible_items(cx);
        let count = visible.len();
        let focus = self.focus_handle.clone();

        v_flex()
            .key_context("ClipboardView")
            .track_focus(&focus)
            .on_action(cx.listener(Self::on_focus_search))
            .on_action(cx.listener(Self::on_copy_selected))
            .on_action(cx.listener(Self::on_delete_selected))
            .on_action(cx.listener(Self::on_select_next))
            .on_action(cx.listener(Self::on_select_prev))
            .size_full()
            .child(self.render_toolbar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        v_flex()
                            .w(px(360.0))
                            .h_full()
                            .border_r_1()
                            .border_color(border)
                            // 列表占满，计数固定在左列底部
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .child(self.render_list(visible, cx)),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .w_full()
                                    .px(px(12.0))
                                    .py(px(6.0))
                                    .border_t_1()
                                    .border_color(border)
                                    .text_xs()
                                    .text_color(muted)
                                    .child(format!("{count} 条")),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(if self.show_settings {
                                self.render_settings(cx).into_any_element()
                            } else {
                                self.render_detail(cx).into_any_element()
                            }),
                    ),
            )
    }
}

impl ClipboardView {
    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;

        h_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(border)
            // 左段：对齐左侧列表列宽（360），搜索框 + 设置；竖线与下方 list/detail 分界对齐
            .child(
                h_flex()
                    .w(px(360.0))
                    .flex_none()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(12.0))
                    .py(px(8.0))
                    .border_r_1()
                    .border_color(border)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.search).small()),
                    )
                    .child(
                        Button::new("clip-settings")
                            .ghost()
                            .small()
                            .icon(icons::settings())
                            .selected(self.show_settings)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.show_settings = !this.show_settings;
                                cx.notify();
                            })),
                    ),
            )
            // 右段：类型筛选 chips（对齐详情列）
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .px(px(12.0))
                    .py(px(8.0))
                    .child(self.render_filter_chips(cx)),
            )
    }

    /// 类型筛选 chips：全部 + 各 ClipKind
    fn render_filter_chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let accent = theme.accent;
        let muted = theme.muted_foreground;
        let secondary = theme.secondary;

        let mut row = h_flex().items_center().gap(px(4.0));
        let chip = |label: SharedString, active: bool| {
            let mut tint = accent;
            tint.a = 0.15;
            div()
                .px(px(8.0))
                .py(px(3.0))
                .rounded_md()
                .text_xs()
                .when(active, |d| d.bg(tint).text_color(accent))
                .when(!active, |d| d.bg(secondary).text_color(muted))
                .child(label)
        };

        // 全部
        row = row.child(
            div()
                .id("filter-all")
                .cursor_pointer()
                .child(chip("全部".into(), self.filter.is_none()))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.filter = None;
                    // 切换筛选回到列表浏览，关闭设置面板
                    this.show_settings = false;
                    cx.notify();
                })),
        );
        for &kind in ClipKind::all() {
            let active = self.filter == Some(kind);
            row = row.child(
                div()
                    .id(SharedString::from(format!("filter-{}", kind.label())))
                    .cursor_pointer()
                    .child(chip(kind.label().into(), active))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.filter = Some(kind);
                        this.show_settings = false;
                        cx.notify();
                    })),
            );
        }
        row
    }

    fn render_list(
        &self,
        visible: Vec<ramag_domain::entities::ClipItem>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if visible.is_empty() {
            let muted = cx.theme().muted_foreground;
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(muted)
                .child("暂无剪贴历史")
                .into_any_element();
        }

        let count = visible.len();
        let entity = cx.entity().clone();
        uniform_list("clip-list", count, move |range, _window, cx| {
            range
                .map(|ix| {
                    entity.update(cx, |this, cx| {
                        this.render_card(&visible[ix], cx).into_any_element()
                    })
                })
                .collect::<Vec<_>>()
        })
        .track_scroll(&self.list_scroll)
        .size_full()
        .into_any_element()
    }
}

// —— Action handlers ——
impl ClipboardView {
    fn on_focus_search(
        &mut self,
        _: &FocusClipSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search.update(cx, |s, cx| s.focus(window, cx));
    }

    fn on_copy_selected(&mut self, _: &CopySelectedClip, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selected(cx);
    }

    fn on_delete_selected(
        &mut self,
        _: &DeleteSelectedClip,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_selected(window, cx);
    }

    fn on_select_next(&mut self, _: &SelectNextClip, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn on_select_prev(&mut self, _: &SelectPrevClip, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, cx);
    }
}
