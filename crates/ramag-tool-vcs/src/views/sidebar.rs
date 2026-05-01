//! 左侧边栏入口：原侧栏 panel 已删除（IDEA 三栏 history pane 接管），
//! 本文件保留 SidebarSection 折叠枚举 + section_header 给三栏左栏复用
//!
//! `render_sidebar` / `SIDEBAR_WIDTH` / `Stash`、`RemoteRepo` 变体保留给将来扩展用。

#![allow(dead_code)]

use gpui::{
    AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable as _, h_flex, scroll::ScrollableElement as _, v_flex,
};

use super::vcs_view::VcsView;

/// Sidebar 总宽度
pub(super) const SIDEBAR_WIDTH: f32 = 260.0;

/// 折叠段标识（用于 section_header 点击切换状态）
#[derive(Debug, Clone, Copy)]
pub(super) enum SidebarSection {
    Local,
    Remote,
    Stash,
    Tag,
    /// 远程仓库配置（与 Remote 分支段不同——这里管的是 remote 本身）
    RemoteRepo,
}

impl VcsView {
    /// 主入口：渲染整条左侧边栏
    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let border = theme.border;
        let bg = theme.sidebar;

        let body: AnyElement = if self.repo.is_none() {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .px(px(12.0))
                .text_xs()
                .text_color(theme.muted_foreground)
                .child("尚未打开仓库")
                .into_any_element()
        } else {
            v_flex()
                .id("vcs-sidebar-scroll")
                .size_full()
                .gap(px(8.0))
                .px(px(8.0))
                .py(px(8.0))
                .overflow_y_scrollbar()
                .child(self.render_local_branches_section(cx))
                .child(self.render_remote_branches_section(cx))
                .child(self.render_stash_section(cx))
                .child(self.render_tags_section(cx))
                .child(self.render_remote_repo_section(cx))
                .into_any_element()
        };

        v_flex()
            .flex_none()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .border_r_1()
            .border_color(border)
            .bg(bg)
            .child(body)
            .into_any_element()
    }
}

/// 段落标题：[▼/▶] 名称 (count) — 整行可点击折叠
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
            SidebarSection::Stash => "stash",
            SidebarSection::Tag => "tag",
            SidebarSection::RemoteRepo => "remote-repo",
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
                SidebarSection::Stash => this.collapsed_stash = !this.collapsed_stash,
                SidebarSection::Tag => this.collapsed_tag = !this.collapsed_tag,
                SidebarSection::RemoteRepo => {
                    this.collapsed_remote_section = !this.collapsed_remote_section;
                }
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
