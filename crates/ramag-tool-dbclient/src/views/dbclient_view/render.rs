//! DbClientView 渲染：顶部连接 Tab Bar + 中心内容（picker / session）

use gpui::{
    AnyView, ClickEvent, Context, IntoElement, ParentElement, Render, SharedString, Styled, Window,
    div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};

use super::{CenterMode, DbClientView};

impl Render for DbClientView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let border = theme.border;
        let secondary_bg = theme.secondary;
        let muted_bg = theme.muted;
        let accent = theme.accent;
        let bg = theme.background;

        let active = self.active_session;

        // (idx, 连接名, 类型, 选中)
        let session_titles: Vec<(usize, String, &'static str, bool)> = self
            .sessions
            .iter()
            .enumerate()
            .map(|(i, s)| {
                (
                    i,
                    s.title(cx).to_string(),
                    s.kind_label(cx),
                    Some(i) == active,
                )
            })
            .collect::<Vec<_>>();

        let on_picker_active = matches!(self.center, CenterMode::ConnectionPicker);

        let mut tab_bar = h_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(border)
            .bg(secondary_bg);

        // 固定 tab：数据源管理
        let picker_btn_active = on_picker_active;
        let mut picker_tab = h_flex()
            .id("picker-tab")
            .items_center()
            .gap_2()
            .px_3()
            .py(px(7.0))
            .border_r_1()
            .border_color(border)
            .cursor_pointer()
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.show_picker(cx);
            }))
            .child(
                ramag_ui::icons::database()
                    .small()
                    .text_color(if picker_btn_active { fg } else { muted_fg }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(if picker_btn_active { fg } else { muted_fg })
                    .child("数据源管理"),
            );

        if picker_btn_active {
            let mut active_bg = accent;
            active_bg.a = 0.15;
            picker_tab = picker_tab.bg(active_bg);
        } else {
            picker_tab = picker_tab.hover(move |this| this.bg(muted_bg));
        }
        tab_bar = tab_bar.child(picker_tab);

        // 右侧 session tabs 横向滚动，不挤压 picker tab
        let mut session_strip = h_flex()
            .id("conn-tabs-scroll")
            .flex_1()
            .min_w_0()
            .overflow_x_scroll()
            .track_scroll(&self.sessions_scroll);

        for (idx, title, kind_label, is_active) in session_titles {
            let tab_id = SharedString::from(format!("conn-tab-{idx}"));
            let close_id = SharedString::from(format!("conn-tab-close-{idx}"));

            // 连接指示点（固定绿）
            let dot_color = gpui::hsla(120.0 / 360.0, 0.5, 0.5, 1.0);

            let mut tab = h_flex()
                .id(tab_id)
                .flex_none()
                .items_center()
                .gap_2()
                .px_3()
                .py(px(7.0))
                .border_r_1()
                .border_color(border)
                .cursor_pointer()
                .child(div().w(px(8.0)).h(px(8.0)).rounded_full().bg(dot_color))
                .child(
                    div()
                        .text_xs()
                        .text_color(if is_active { fg } else { muted_fg })
                        .child(title.clone()),
                )
                .child(div().text_xs().text_color(muted_fg).child(kind_label))
                .child(
                    Button::new(close_id)
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.close_session(idx, cx);
                        })),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.select_session(idx, window, cx);
                }));

            if is_active && !on_picker_active {
                let mut active_bg = accent;
                active_bg.a = 0.15;
                tab = tab.bg(active_bg);
            } else {
                tab = tab.hover(move |this| this.bg(muted_bg));
            }

            session_strip = session_strip.child(tab);
        }

        tab_bar = tab_bar.child(session_strip);

        let center_view: AnyView = match &self.center {
            CenterMode::Session => match active.and_then(|i| self.sessions.get(i)) {
                Some(s) => s.to_any_view(),
                None => self.picker.clone().into(),
            },
            CenterMode::ConnectionPicker => self.picker.clone().into(),
        };

        v_flex()
            .size_full()
            .bg(bg)
            .text_color(fg)
            .child(tab_bar)
            .child(div().flex_1().min_h_0().child(center_view))
    }
}
