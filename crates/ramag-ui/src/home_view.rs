//! 首页：ANSI Shadow Logo + tagline（工具入口在左侧 ActivityBar）

use std::sync::Arc;

use gpui::{
    Context, EventEmitter, IntoElement, ParentElement, Render, SharedString, Styled, Window, div,
    hsla, px,
};
use gpui_component::{ActiveTheme, scroll::ScrollableElement as _, v_flex};

use ramag_app::{ConnectionService, ToolRegistry};
use ramag_domain::entities::ConnectionId;

#[derive(Debug, Clone)]
pub enum HomeEvent {
    OpenTool(String),
    OpenConnection(ConnectionId),
}

/// ANSI Shadow 大字，等宽对齐
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
        let accent = theme.accent;
        let mono = theme.mono_font_family.clone();
        let bg = theme.background;

        v_flex().size_full().bg(bg).overflow_y_scrollbar().child(
            v_flex().size_full().items_center().pt(px(96.0)).child(
                v_flex()
                    .w_full()
                    .max_w(px(840.0))
                    .px(px(40.0))
                    .child(render_logo(mono, accent, muted_fg)),
            ),
        )
    }
}

fn render_logo(mono: SharedString, accent: gpui::Hsla, muted_fg: gpui::Hsla) -> impl IntoElement {
    // 顶部稍亮往下逐行掉 alpha 做层次
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
