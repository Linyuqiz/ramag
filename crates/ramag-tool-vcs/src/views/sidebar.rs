//! 折叠段共享件：SidebarSection + section_header + history 左栏行类型 / 分发。
//! 左栏（本地/远程分支 + Tag）合并为单个 uniform_list，所有行统一 28px 等高

use gpui::{
    AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, div, prelude::*, px,
};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable as _, h_flex};
use ramag_domain::entities::{Branch, Tag};

use super::vcs_view::VcsView;

/// 行高固定 28px：uniform_list 行级虚拟化要求所有行等高
pub(super) const LEFT_ROW_H: f32 = 28.0;

/// 折叠段标识（用于 section_header 点击切换状态）
#[derive(Debug, Clone, Copy)]
pub(super) enum SidebarSection {
    Local,
    Remote,
    Tag,
}

/// 左栏扁平行：段表头 / 分支 / Tag / 新建输入 / 空占位
pub(super) enum LeftRow {
    Header {
        title: &'static str,
        count: usize,
        collapsed: bool,
        section: SidebarSection,
    },
    Branch {
        idx: usize,
        branch: Branch,
        is_remote: bool,
    },
    Tag {
        idx: usize,
        tag: Tag,
    },
    CreateBranch,
    CreateTag,
    Empty(&'static str),
}

impl VcsView {
    /// uniform_list 单行分发（左栏：分支段 + Tag 段）
    pub(super) fn render_left_row(&self, row: &LeftRow, cx: &mut Context<Self>) -> AnyElement {
        match row {
            LeftRow::Header {
                title,
                count,
                collapsed,
                section,
            } => section_header(title, *count, *collapsed, *section, cx),
            LeftRow::Branch {
                idx,
                branch,
                is_remote,
            } => super::sidebar_branches::branch_row(*idx, branch, self.busy, *is_remote, cx)
                .into_any_element(),
            LeftRow::Tag { idx, tag } => {
                super::sidebar_tags::tag_row(*idx, tag, self.busy, cx).into_any_element()
            }
            LeftRow::CreateBranch => self.render_create_branch_row(cx),
            LeftRow::CreateTag => self.render_create_tag_row(cx),
            LeftRow::Empty(msg) => {
                let muted_fg = cx.theme().muted_foreground;
                h_flex()
                    .h(px(LEFT_ROW_H))
                    .flex_none()
                    .items_center()
                    .pl(px(4.0))
                    .text_xs()
                    .text_color(muted_fg)
                    .child(*msg)
                    .into_any_element()
            }
        }
    }
}

/// 段标题：折叠图标 + 名称 + 计数，整行可点折叠（固定 28px 高）
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
        .h(px(LEFT_ROW_H))
        .flex_none()
        .gap(px(4.0))
        .items_center()
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
