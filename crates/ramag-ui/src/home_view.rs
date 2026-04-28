//! Ramag 首页 — 终端风
//!
//! 结构：ANSI Shadow 大字 RAMAG + 一行 tagline + 三模块卡（数据库 / 版本管理 / 终端）。
//! 数据库卡点击直接进入 dbclient（数据库类型选择由 dbclient 内部完成）。

use std::sync::Arc;

use gpui::{
    ClickEvent, Context, EventEmitter, IntoElement, ParentElement, Render, SharedString, Styled,
    Window, div, hsla, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Icon, Sizable as _, h_flex, scroll::ScrollableElement as _, v_flex,
};

use crate::icons;
use ramag_app::{ConnectionService, ToolRegistry};
use ramag_domain::entities::ConnectionId;

#[derive(Debug, Clone)]
pub enum HomeEvent {
    OpenTool(String),
    OpenConnection(ConnectionId),
}

/// ANSI Shadow 风 "RAMAG" 大字（每行等宽，配合 mono 字体显示成连续色块）
const RAMAG_LOGO: &[&str] = &[
    "██████╗  █████╗ ███╗   ███╗ █████╗  ██████╗ ",
    "██╔══██╗██╔══██╗████╗ ████║██╔══██╗██╔════╝ ",
    "██████╔╝███████║██╔████╔██║███████║██║  ███╗",
    "██╔══██╗██╔══██║██║╚██╔╝██║██╔══██║██║   ██║",
    "██║  ██║██║  ██║██║ ╚═╝ ██║██║  ██║╚██████╔╝",
    "╚═╝  ╚═╝╚═╝  ╚═╝╚═╝     ╚═╝╚═╝  ╚═╝ ╚═════╝ ",
];

pub struct HomeView;

impl EventEmitter<HomeEvent> for HomeView {}

impl HomeView {
    pub fn new(
        _registry: Arc<ToolRegistry>,
        _service: Arc<ConnectionService>,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self
    }
}

impl Render for HomeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let accent = theme.accent;
        let secondary_bg = theme.secondary;
        let border = theme.border;
        let card_hover = theme.muted;
        let mono = theme.mono_font_family.clone();
        let bg = theme.background;

        // 外层撑满 + 滚动；内容距顶 96px（视觉居中偏上 1/3）
        v_flex().size_full().bg(bg).overflow_y_scrollbar().child(
            v_flex().size_full().items_center().pt(px(96.0)).child(
                v_flex()
                    .w_full()
                    .max_w(px(840.0))
                    .px(px(40.0))
                    .gap(px(32.0))
                    .child(render_logo(mono, accent, muted_fg))
                    .child(
                        h_flex()
                            .gap(px(14.0))
                            .w_full()
                            .child(active_module_card(
                                "module-db",
                                icons::database(),
                                "数据库",
                                "MySQL · PostgreSQL · Redis · MongoDB",
                                secondary_bg,
                                border,
                                fg,
                                muted_fg,
                                accent,
                                card_hover,
                                cx.listener(|_this, _: &ClickEvent, _, cx| {
                                    cx.emit(HomeEvent::OpenTool("dbclient".into()));
                                }),
                            ))
                            .child(soon_module_card(
                                "module-term",
                                icons::terminal(),
                                "终端",
                                "本地 Shell · SSH",
                                secondary_bg,
                                border,
                                muted_fg,
                            ))
                            .child(soon_module_card(
                                "module-vc",
                                icons::git_branch(),
                                "版本管理",
                                "Git 仓库 · 提交 · 合并",
                                secondary_bg,
                                border,
                                muted_fg,
                            )),
                    ),
            ),
        )
    }
}

/// Logo 区：ANSI Shadow 大字 + tagline（等宽字体居中）
fn render_logo(mono: SharedString, accent: gpui::Hsla, muted_fg: gpui::Hsla) -> impl IntoElement {
    // 渐变叠色：从顶部稍亮往下逐行掉点 alpha，制造层次感
    let mut lines = Vec::with_capacity(RAMAG_LOGO.len());
    for (i, line) in RAMAG_LOGO.iter().enumerate() {
        let alpha = 1.0 - (i as f32) * 0.06;
        let color = hsla(accent.h, accent.s, accent.l, alpha);
        lines.push(
            div()
                .text_color(color)
                .line_height(px(13.0))
                .child(SharedString::from(line.to_string())),
        );
    }

    v_flex()
        .items_center()
        .gap(px(18.0))
        .child(
            v_flex()
                .font_family(mono.clone())
                .text_size(px(14.0))
                .font_weight(gpui::FontWeight::BOLD)
                .children(lines),
        )
        .child(
            div()
                .font_family(mono)
                .text_size(px(12.0))
                .text_color(muted_fg)
                .child(SharedString::from(
                    "$ minimal by design · local by default_",
                )),
        )
}

/// 主模块卡片：可点击，hover 高亮
#[allow(clippy::too_many_arguments)]
fn active_module_card(
    id: &'static str,
    icon: Icon,
    name: &'static str,
    desc: &'static str,
    bg: gpui::Hsla,
    border: gpui::Hsla,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    accent: gpui::Hsla,
    hover_bg: gpui::Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let mut tinted_accent = accent;
    tinted_accent.a = 0.12;

    v_flex()
        .id(SharedString::from(id))
        .flex_1()
        .min_w(px(160.0))
        .gap(px(8.0))
        .p(px(14.0))
        .rounded_md()
        .bg(bg)
        .border_1()
        .border_color(border)
        .cursor_pointer()
        .hover(move |this| this.bg(hover_bg).border_color(accent))
        .on_click(on_click)
        .child(
            div()
                .w(px(30.0))
                .h(px(30.0))
                .rounded(px(6.0))
                .bg(tinted_accent)
                .flex()
                .items_center()
                .justify_center()
                .child(icon.small().text_color(accent)),
        )
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(fg)
                .child(name),
        )
        .child(div().text_xs().text_color(muted_fg).child(desc))
}

/// "Coming Soon" 模块卡片：dim、无 hover、无点击
fn soon_module_card(
    id: &'static str,
    icon: Icon,
    name: &'static str,
    desc: &'static str,
    bg: gpui::Hsla,
    border: gpui::Hsla,
    muted_fg: gpui::Hsla,
) -> impl IntoElement {
    let mut tinted = muted_fg;
    tinted.a = 0.18;

    v_flex()
        .id(SharedString::from(id))
        .flex_1()
        .min_w(px(160.0))
        .gap(px(8.0))
        .p(px(14.0))
        .rounded_md()
        .bg(bg)
        .border_1()
        .border_color(border)
        .opacity(0.55)
        .child(
            div()
                .w(px(30.0))
                .h(px(30.0))
                .rounded(px(6.0))
                .bg(tinted)
                .flex()
                .items_center()
                .justify_center()
                .child(icon.small().text_color(muted_fg)),
        )
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(muted_fg)
                .child(name),
        )
        .child(div().text_xs().text_color(muted_fg).child(desc))
}
