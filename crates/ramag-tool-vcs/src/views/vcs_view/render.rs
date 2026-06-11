//! VcsView 顶层 Render：tab bar + body 路由（RepoList / IDE 布局）

use gpui::{
    AnyElement, Context, Focusable as _, IntoElement, ParentElement, Render, Styled, Window, div,
    prelude::*,
};
use gpui_component::{ActiveTheme, v_flex};

use super::super::helpers::ActiveView;
use super::VcsView;

impl Render for VcsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 把异步操作完成时挂起的 toast 推送出来（commit / push / pull 等成功提示）
        if let Some(n) = self.pending_notification.take() {
            use gpui_component::WindowExt as _;
            window.push_notification(n, cx);
        }
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
            // CloseTab：有 active file tab 关它，否则冒泡到全局 fallback 关窗
            .on_action(cx.listener(|this, _: &ramag_ui::CloseTab, window, cx| {
                if let Some(idx) = this.active_file_tab_idx {
                    this.close_file_tab(idx, cx);
                    window.focus(&this.focus_handle, cx);
                } else {
                    cx.propagate();
                }
            }))
            // cmd-r：手动刷新工作区
            .on_action(
                cx.listener(|this, _: &crate::actions::RefreshWorkspace, _, cx| {
                    if this.repo.is_some() && !this.loading {
                        this.refresh_workspace_silent(cx);
                    }
                }),
            )
            // cmd-shift-k / cmd-t：push / pull 当前分支
            .on_action(cx.listener(|this, _: &crate::actions::PushNow, _, cx| {
                if this.repo.is_some() && !this.busy {
                    this.run_remote_op(super::super::helpers::RemoteOp::Push, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &crate::actions::PullNow, _, cx| {
                if this.repo.is_some() && !this.busy {
                    this.run_remote_op(super::super::helpers::RemoteOp::Pull, cx);
                }
            }))
            // cmd-shift-h：底部历史面板
            .on_action(
                cx.listener(|this, _: &crate::actions::ToggleHistoryPane, _, cx| {
                    if this.repo.is_some() {
                        this.toggle_history_pane(cx);
                    }
                }),
            )
            // cmd-k：切 Changes 并聚焦 commit 输入框
            .on_action(
                cx.listener(|this, _: &crate::actions::FocusCommitMessage, window, cx| {
                    if this.repo.is_none() {
                        return;
                    }
                    this.set_files_view_mode(super::super::helpers::FilesViewMode::Changes, cx);
                    let fh = this.commit_input.read(cx).focus_handle(cx);
                    window.focus(&fh, cx);
                }),
            )
            // cmd-enter：仅 commit 输入框聚焦时提交（其他输入框里不劫持）
            .on_action(
                cx.listener(|this, _: &crate::actions::CommitNow, window, cx| {
                    if this.repo.is_none() || this.busy {
                        return;
                    }
                    let fh = this.commit_input.read(cx).focus_handle(cx);
                    if fh.is_focused(window) {
                        this.confirm_commit(window, cx);
                    } else {
                        cx.propagate();
                    }
                }),
            )
            .child(self.render_tabs(cx))
            .child(div().flex_1().min_h_0().child(body))
    }
}
