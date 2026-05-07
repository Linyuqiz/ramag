//! 折叠段共享件：SidebarSection + section_header，由 history_panel 左栏复用

use gpui::{
    AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, div, prelude::*, px,
};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable as _, h_flex};

use super::vcs_view::VcsView;

/// 折叠段标识（用于 section_header 点击切换状态）
#[derive(Debug, Clone, Copy)]
pub(super) enum SidebarSection {
    Local,
    Remote,
    Tag,
}

/// 段标题：折叠图标 + 名称 + 计数，整行可点折叠
pub(super) fn section_header(
    title: &'static str,
    count: usize,
    collapsed: bool,
    sec: SidebarSection,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let theme = cx.theme();
    let muted_fg = theme.muted_foreground;
    let chev = if collapsed {
        IconName::ChevronRight
    } else {
        IconName::ChevronDown
    };
    let id = SharedString::from(format!(
        "vcs-side-section-{}",
        match sec {
            SidebarSection::Local => "local",
            SidebarSection::Remote => "remote",
            SidebarSection::Tag => "tag",
        }
    ));
    let hover_bg = theme.muted;

    h_flex()
        .id(id)
        .gap(px(4.0))
        .items_center()
        .py(px(3.0))
        .px(px(2.0))
        .rounded(px(3.0))
        .cursor_pointer()
        .hover(move |this| this.bg(hover_bg))
        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
            match sec {
                SidebarSection::Local => this.collapsed_local = !this.collapsed_local,
                SidebarSection::Remote => this.collapsed_remote = !this.collapsed_remote,
                SidebarSection::Tag => this.collapsed_tag = !this.collapsed_tag,
            }
            cx.notify();
        }))
        .child(Icon::new(chev).xsmall().text_color(muted_fg))
        .child(
            div()
                .flex_1()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(muted_fg)
                .child(format!("{title} ({count})")),
        )
        .into_any_element()
}
