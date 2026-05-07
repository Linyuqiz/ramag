//! 顶部 Tab Bar：固定「仓库管理」tab + 每仓一个 tab（×=关，全关后回管理页）

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};

use super::helpers::ActiveView;
use super::vcs_view::VcsView;

impl VcsView {
    /// 渲染顶部 Tab Bar
    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted_fg = theme.muted_foreground;
        let border = theme.border;
        let mut tab_bar_bg = theme.sidebar;
        tab_bar_bg.l = (tab_bar_bg.l + 0.01).min(1.0);
        let muted_bg = theme.muted;
        let accent = theme.accent;
        let mut accent_bg = theme.accent;
        accent_bg.a = 0.15;
        let on_list = matches!(self.active_view, ActiveView::RepoList);

        let mut bar = h_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(border)
            .bg(tab_bar_bg);

        // [仓库管理] 固定 tab：点击回管理页，不影响已打开的仓库
        let mut list_tab = h_flex()
            .id("vcs-tab-repo-list")
            .items_center()
            .gap(px(6.0))
            .px(px(12.0))
            .py(px(7.0))
            .border_r_1()
            .border_color(border)
            .cursor_pointer()
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.show_repo_list(cx);
            }))
            .child(
                Icon::new(ramag_ui::icons::git_branch())
                    .small()
                    .text_color(if on_list { fg } else { muted_fg }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(if on_list { fg } else { muted_fg })
                    .child("仓库管理"),
            );
        if on_list {
            list_tab = list_tab.bg(accent_bg);
        } else {
            list_tab = list_tab.hover(move |this| this.bg(muted_bg));
        }
        bar = bar.child(list_tab);

        // 每个已打开仓库对应一个 tab
        for repo in &self.open_repos {
            let is_active = !on_list
                && self
                    .repo
                    .as_ref()
                    .map(|r| r.path == repo.path)
                    .unwrap_or(false);
            let path_switch = repo.path.clone();
            let path_close = repo.path.clone();
            let name = SharedString::from(repo.name.clone());
            let tab_id = SharedString::from(format!("vcs-tab-repo-{}", repo.path));
            let label_id = SharedString::from(format!("vcs-tab-label-{}", repo.path));
            let close_id = SharedString::from(format!("vcs-tab-close-{}", repo.path));

            // 外层无 on_click，内层标签区单独响应切换，关闭按钮独立，避免事件冒泡冲突
            let mut tab = h_flex()
                .id(tab_id)
                .items_center()
                .border_r_1()
                .border_color(border)
                .pr(px(4.0))
                .child(
                    h_flex()
                        .id(label_id)
                        .items_center()
                        .gap(px(6.0))
                        .px(px(12.0))
                        .py(px(7.0))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if this
                                .repo
                                .as_ref()
                                .map(|r| r.path == path_switch)
                                .unwrap_or(false)
                            {
                                this.active_view = ActiveView::Session;
                                cx.notify();
                            } else {
                                this.open_recent_repo(path_switch.clone(), cx);
                            }
                        }))
                        .child(div().w(px(8.0)).h(px(8.0)).rounded_full().bg(accent))
                        .child(
                            div()
                                .text_xs()
                                .text_color(if is_active { fg } else { muted_fg })
                                .child(name),
                        ),
                )
                .child(
                    Button::new(close_id)
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.remove_open_repo(path_close.clone(), cx);
                        })),
                );
            if is_active {
                tab = tab.bg(accent_bg);
            } else {
                tab = tab.hover(move |this| this.bg(muted_bg));
            }
            bar = bar.child(tab);
        }

        bar.into_any_element()
    }
}
