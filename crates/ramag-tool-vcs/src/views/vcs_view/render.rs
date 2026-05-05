//! VcsView 顶层 Render：tab bar + body 路由（RepoList / IDE 布局）

use gpui::{
    AnyElement, Context, IntoElement, ParentElement, Render, Styled, Window, div, prelude::*,
};
use gpui_component::{ActiveTheme, v_flex};

use super::super::helpers::ActiveView;
use super::VcsView;

impl Render for VcsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // commit 草稿恢复：仓库切换后用 cx.defer_in 借 Window 写回 InputState
        if let Some(text) = self.pending_commit_text.take() {
            let input = self.commit_input.clone();
            cx.defer_in(window, move |_, window, cx| {
                input.update(cx, |state, ctx| {
                    state.set_value(text, window, ctx);
                });
            });
        }
        let theme = cx.theme();
        let bg = theme.background;
        let muted_fg = theme.muted_foreground;

        // 两层结构（仿 dbclient）：tab bar（含右侧操作区） / body
        // body 由 active_view 路由：RepoList → 仓库管理页；Session → IDE 布局
        // 注意：error 不再独占 body —— 由 RepoList 顶部 banner 承载（不阻塞用户操作）
        let body: AnyElement = if self.loading {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(muted_fg)
                .child("加载中...")
                .into_any_element()
        } else {
            match self.active_view {
                ActiveView::RepoList => self.render_repo_list(cx),
                ActiveView::Session => {
                    if self.repo.is_some() {
                        self.render_ide_layout(cx)
                    } else {
                        // 异常态：active_view=Session 但 repo 不存在 → fallback 列表
                        self.render_repo_list(cx)
                    }
                }
            }
        };

        v_flex()
            .size_full()
            .bg(bg)
            .key_context("VcsView")
            .track_focus(&self.focus_handle)
            // ⌘W：有 active file tab 时关闭它；否则把事件冒泡到全局 fallback（关窗）
            .on_action(cx.listener(|this, _: &ramag_ui::CloseTab, window, cx| {
                if let Some(idx) = this.active_file_tab_idx {
                    this.close_file_tab(idx, cx);
                    window.focus(&this.focus_handle, cx);
                } else {
                    cx.propagate();
                }
            }))
            .child(self.render_tabs(cx))
            .child(div().flex_1().min_h_0().child(body))
    }
}
