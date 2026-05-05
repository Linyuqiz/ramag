//! ConnectionListPanel 渲染：header（搜索 + 新建按钮）+ body（行列表 / 空状态）

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, ParentElement, Render, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    scroll::ScrollableElement as _,
    v_flex,
};

use super::row::connection_row;
use super::{ConnectionListPanel, ListEvent};

impl Render for ConnectionListPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let accent = theme.accent;
        let border = theme.border;
        let row_hover = theme.muted;
        let bg = theme.background;

        let total = self.connections.len();
        let loading = self.loading;
        let visible = self.filtered();
        let visible_count = visible.len();
        let selected = self.selected.clone();

        // 内容统一限制最大宽度 1080px 居中，避免大屏摊得太开
        // 头部和列表行用同一个容器宽度，左右对齐整齐
        const CONTENT_MAX_W: f32 = 1080.0;

        // ===== Header =====
        // 极简布局：左侧搜索框（max 360px）+ 右侧"新建连接"（outline + small，更克制）
        let header_inner = h_flex()
            .w_full()
            .items_center()
            .gap(px(16.0))
            .child(
                div().flex_1().min_w_0().child(
                    div().max_w(px(360.0)).child(
                        Input::new(&self.search)
                            .small()
                            .cleanable(true)
                            .prefix(Icon::new(IconName::Search).small().text_color(muted_fg)),
                    ),
                ),
            )
            .child(
                Button::new("add-connection")
                    .outline()
                    .small()
                    .icon(IconName::Plus)
                    .tooltip("新建连接")
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        cx.emit(ListEvent::RequestNew);
                    })),
            );

        // 顶部和 tab bar 之间留出呼吸空间（pt 比 pb 略大）
        let header = h_flex()
            .w_full()
            .justify_center()
            .px(px(24.0))
            .pt(px(22.0))
            .pb(px(16.0))
            .border_b_1()
            .border_color(border)
            .child(div().w_full().max_w(px(CONTENT_MAX_W)).child(header_inner));

        // ===== Body =====
        let body: AnyElement = if loading {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(div().text_sm().text_color(muted_fg).child("加载中..."))
                .into_any_element()
        } else if total == 0 {
            empty_state(border, muted_fg, fg, accent, cx).into_any_element()
        } else if visible_count == 0 {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .child(
                    div()
                        .text_sm()
                        .text_color(fg)
                        .child(format!("没有匹配「{}」的连接", self.query)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted_fg)
                        .child("尝试修改关键字或清空搜索"),
                )
                .into_any_element()
        } else {
            let mut rows: Vec<AnyElement> = Vec::with_capacity(visible_count);
            for (idx, conn) in visible.into_iter().enumerate() {
                let is_selected = selected.as_ref() == Some(&conn.id);
                let version = self.versions.get(&conn.id).cloned();
                rows.push(
                    connection_row(
                        idx,
                        conn,
                        is_selected,
                        version,
                        border,
                        row_hover,
                        accent,
                        fg,
                        muted_fg,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            v_flex()
                .size_full()
                .overflow_y_scrollbar()
                .child(
                    h_flex()
                        .w_full()
                        .justify_center()
                        .px(px(24.0))
                        .py(px(10.0))
                        .child(v_flex().w_full().max_w(px(CONTENT_MAX_W)).children(rows)),
                )
                .into_any_element()
        };

        v_flex().size_full().bg(bg).child(header).child(body)
    }
}

/// 空状态：一个大引导块，主按钮"新建连接"
fn empty_state(
    border: gpui::Hsla,
    muted_fg: gpui::Hsla,
    fg: gpui::Hsla,
    accent: gpui::Hsla,
    cx: &mut Context<ConnectionListPanel>,
) -> impl IntoElement {
    let mut tinted_accent = accent;
    tinted_accent.a = 0.12;

    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap(px(20.0))
        .child(
            div()
                .w(px(64.0))
                .h(px(64.0))
                .rounded(px(14.0))
                .bg(tinted_accent)
                .flex()
                .items_center()
                .justify_center()
                .child(ramag_ui::icons::database().text_color(accent)),
        )
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(fg)
                .child("还没有连接"),
        )
        .child(
            div()
                .text_sm()
                .text_color(muted_fg)
                .child("点击下方按钮创建第一个数据库连接"),
        )
        .child(
            Button::new("empty-add")
                .primary()
                .icon(IconName::Plus)
                .label("新建连接")
                .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                    cx.emit(ListEvent::RequestNew);
                })),
        )
        .pb(px(64.0))
        .pt(px(64.0))
        .mx(px(40.0))
        .border_1()
        .border_color(border)
        .rounded_lg()
}
