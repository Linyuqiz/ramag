//! 破坏性操作的二次确认对话框集中地
//!
//! 设计原则：
//! - **统一入口**：click handler 都调 `confirm_xxx(op, window, cx)`，由本文件分发；
//!   非破坏性 op 直接转发到原 `run_xxx`，破坏性 op 弹 dialog 等用户确认
//! - **公共 helper**：[`open_confirm_dialog`] 抽取自最早的 `confirm_remove_recent_repo`
//! - **danger 颜色**：危险按钮（删 / 强推 / 丢工作）用红；中等危险（合并 / rebase / amend）用 primary
//!
//! 危险等级判定见 README 风险表。

use gpui::{ClickEvent, Context, Entity, ParentElement, SharedString, Styled, Window, div, px};
use gpui_component::{
    ActiveTheme, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};
use ramag_domain::entities::RepoOperation;

use super::helpers::{BranchOp, FileOp, RemoteOp, StashOp, TagOp};
use super::vcs_view::VcsView;
use super::vcs_view_ops_remote::RemoteAdminOp;

/// 通用二次确认对话框
///
/// `danger=true` 时确认按钮用红色；`false` 时用 primary 蓝色。
/// `on_confirm` 在用户点确认后跑（已经在 `view.update` 里，能直接 mutate VcsView）
#[allow(clippy::too_many_arguments)]
pub(super) fn open_confirm_dialog(
    view: Entity<VcsView>,
    title: impl Into<SharedString>,
    description: String,
    confirm_label: impl Into<SharedString>,
    danger: bool,
    on_confirm: impl FnOnce(&mut VcsView, &mut Context<VcsView>) + 'static,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let title: SharedString = title.into();
    let confirm_label: SharedString = confirm_label.into();
    // on_confirm 是 FnOnce 但 open_dialog 接 Fn —— 闭包外先包成 RefCell+Rc，
    // 闭包内 clone Rc，clone 出来的本身是 Fn（每次调用 take Option 内的 callback）
    let on_confirm_cell = std::rc::Rc::new(std::cell::RefCell::new(Some(on_confirm)));
    window.open_dialog(cx, move |dialog, _, _| {
        let view = view.clone();
        let desc = description.clone();
        let confirm_label_inner = confirm_label.clone();

        let cancel_btn = Button::new("vcs-confirm-cancel")
            .ghost()
            .small()
            .label("取消")
            .on_click(|_: &ClickEvent, window, app| {
                window.close_dialog(app);
            });

        let mut ok_btn = Button::new("vcs-confirm-ok")
            .small()
            .label(confirm_label_inner);
        ok_btn = if danger {
            ok_btn.danger()
        } else {
            ok_btn.primary()
        };

        let ok_btn = ok_btn.on_click({
            let view = view.clone();
            let cell = on_confirm_cell.clone();
            move |_: &ClickEvent, window, app| {
                if let Some(cb) = cell.borrow_mut().take() {
                    view.update(app, |this, cx| cb(this, cx));
                }
                window.close_dialog(app);
            }
        });

        dialog
            .title(title.clone())
            .margin_top(px(180.0))
            .content(move |content, _, cx| {
                let muted_fg = cx.theme().muted_foreground;
                content.child(
                    div()
                        .py(px(4.0))
                        .text_sm()
                        .text_color(muted_fg)
                        .child(desc.clone()),
                )
            })
            .footer(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_end()
                    .gap(px(8.0))
                    .child(cancel_btn)
                    .child(ok_btn),
            )
    });
}

impl VcsView {
    /// File 操作（Stage / Unstage / Discard）；只有 Discard 弹确认
    pub(super) fn confirm_file_op(
        &mut self,
        op: FileOp,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(op, FileOp::Discard) {
            self.run_file_op(op, path, cx);
            return;
        }
        let view = cx.entity();
        let path_for_run = path.clone();
        open_confirm_dialog(
            view,
            "丢弃工作区改动？",
            format!("将丢弃「{path}」在工作区的全部未暂存改动，且无法恢复。\n确认继续吗？"),
            "丢弃",
            true,
            move |this, cx| this.run_file_op(FileOp::Discard, path_for_run, cx),
            window,
            cx,
        );
    }

    /// Stash 操作；Drop 弹确认（Save / Apply / Pop 不弹——内容仍在工作区可恢复）
    pub(super) fn confirm_stash_op(
        &mut self,
        op: StashOp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let StashOp::Drop(idx) = op else {
            self.run_stash_op(op, cx);
            return;
        };
        let stash_msg = self
            .stashes
            .get(idx)
            .map(|s| s.message.clone())
            .unwrap_or_else(|| format!("stash@{{{idx}}}"));
        let view = cx.entity();
        open_confirm_dialog(
            view,
            "删除 stash？",
            format!("将永久删除 stash「{stash_msg}」，无法恢复。\n确认继续吗？"),
            "删除",
            true,
            move |this, cx| this.run_stash_op(StashOp::Drop(idx), cx),
            window,
            cx,
        );
    }

    /// Remote 同步操作；只有 PushForce 弹确认（fetch/pull/push 普通操作不弹）
    pub(super) fn confirm_remote_op(
        &mut self,
        op: RemoteOp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(op, RemoteOp::PushForce) {
            self.run_remote_op(op, cx);
            return;
        }
        let view = cx.entity();
        open_confirm_dialog(
            view,
            "强制推送？",
            "git push --force-with-lease 会改写远程分支历史。\n\
             仅当远程的 ref 仍指向你预期的 commit 时才覆盖（比 --force 安全），\
             但仍可能让其他基于此分支工作的协作者丢失提交。\n确认继续吗？"
                .into(),
            "强推",
            true,
            move |this, cx| this.run_remote_op(RemoteOp::PushForce, cx),
            window,
            cx,
        );
    }

    /// 分支操作；Delete / Merge / Rebase 弹确认（Checkout / Create 不弹）
    pub(super) fn confirm_branch_op(
        &mut self,
        op: BranchOp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (title, desc, btn, danger) = match &op {
            BranchOp::Delete(name, true) => (
                "强制删除分支？",
                format!(
                    "分支「{name}」可能还有未合并的提交，强制删除后这些提交不再可达，可能丢失。\n\
                     确认继续吗？"
                ),
                "强制删除",
                true,
            ),
            BranchOp::Delete(name, false) => (
                "删除分支？",
                format!("将删除本地分支「{name}」（仅当已合并；未合并会报错）。\n确认继续吗？"),
                "删除",
                true,
            ),
            BranchOp::Merge(name) => (
                "合并分支？",
                format!(
                    "将「{name}」合并到当前 HEAD（默认 --no-ff，建 merge commit）。\n\
                     有冲突时会进入合并进行中状态，需手动解决后再继续。"
                ),
                "合并",
                false,
            ),
            BranchOp::Rebase(name) => (
                "Rebase 到目标分支？",
                format!(
                    "将当前分支 rebase 到「{name}」上：\n\
                     - 改写当前分支的 commit 历史\n\
                     - 已 push 的分支谨慎使用（推送时需要 --force-with-lease）\n\
                     有冲突时会进入 rebase 进行中状态。"
                ),
                "Rebase",
                false,
            ),
            BranchOp::Checkout(name) => {
                // dirty 检测：staged 或 unstaged 非空（untracked 不阻止 checkout）
                let dirty = self
                    .status
                    .as_ref()
                    .map(|s| {
                        s.files
                            .iter()
                            .any(|f| f.staged.is_some() || f.unstaged.is_some())
                    })
                    .unwrap_or(false);
                if dirty {
                    open_checkout_dirty_dialog(cx.entity(), name.clone(), window, cx);
                } else {
                    self.run_branch_op(op, cx);
                }
                return;
            }
            _ => {
                self.run_branch_op(op, cx);
                return;
            }
        };
        let view = cx.entity();
        open_confirm_dialog(
            view,
            title,
            desc,
            btn,
            danger,
            move |this, cx| this.run_branch_op(op, cx),
            window,
            cx,
        );
    }

    /// Tag 操作；Delete / Push 弹确认（Create 不弹）
    pub(super) fn confirm_tag_op(
        &mut self,
        op: TagOp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (title, desc, btn, danger) = match &op {
            TagOp::Delete(name) => (
                "删除 tag？",
                format!("将删除本地 tag「{name}」。已推送的 tag 删除后远程仍存在。\n确认继续吗？"),
                "删除",
                true,
            ),
            TagOp::Push(name) => (
                "推送 tag 到远程？",
                format!("将把 tag「{name}」推送到 origin。\n推送后此 tag 对所有协作者可见。"),
                "推送",
                false,
            ),
            _ => {
                self.run_tag_op(op, cx);
                return;
            }
        };
        let view = cx.entity();
        open_confirm_dialog(
            view,
            title,
            desc,
            btn,
            danger,
            move |this, cx| this.run_tag_op(op, cx),
            window,
            cx,
        );
    }

    /// Remote 配置操作；Remove 弹确认（Add / SetUrl 不弹）
    #[allow(dead_code)]
    pub(super) fn confirm_remote_admin_op(
        &mut self,
        op: RemoteAdminOp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let RemoteAdminOp::Remove(name) = &op else {
            self.run_remote_admin_op(op, cx);
            return;
        };
        let name_owned = name.clone();
        let view = cx.entity();
        open_confirm_dialog(
            view,
            "删除 remote？",
            format!("将删除 remote「{name_owned}」配置（不影响远程仓库本身）。\n确认继续吗？"),
            "删除",
            true,
            move |this, cx| this.run_remote_admin_op(op, cx),
            window,
            cx,
        );
    }

    /// 进行中操作的步进；Abort 弹确认（Continue / Skip 不弹）
    pub(super) fn confirm_op_step(
        &mut self,
        step: super::helpers::OperationStep,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use super::helpers::OperationStep;
        if !matches!(step, OperationStep::Abort) {
            self.run_op_step(step, cx);
            return;
        }
        let op_name = self
            .status
            .as_ref()
            .and_then(|s| s.operation)
            .map(|o| match o {
                RepoOperation::Merge => "合并",
                RepoOperation::Rebase => "Rebase",
                RepoOperation::CherryPick => "Cherry-pick",
                RepoOperation::Revert => "Revert",
            })
            .unwrap_or("当前操作");
        let view = cx.entity();
        open_confirm_dialog(
            view,
            "中止当前操作？",
            format!(
                "将中止进行中的「{op_name}」并回到操作前的状态。\n\
                 已解决一半的冲突会被丢弃，无法恢复。"
            ),
            "中止",
            true,
            move |this, cx| this.run_op_step(OperationStep::Abort, cx),
            window,
            cx,
        );
    }

    /// Commit：amend=true 时弹确认（普通 commit 不弹）
    pub(super) fn confirm_commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.commit_amend {
            self.run_commit(cx);
            return;
        }
        let view = cx.entity();
        open_confirm_dialog(
            view,
            "Amend 上一次提交？",
            "将用当前暂存区改动 + 新 message 替换最近一次 commit。\n\
             如果该 commit 已经 push 到远程，再次推送需要 --force-with-lease。"
                .into(),
            "Amend 提交",
            false,
            move |this, cx| this.run_commit(cx),
            window,
            cx,
        );
    }
}

/// Checkout 时工作区有未提交改动 → 三选一对话框：取消 / Stash 后切换 / 丢弃后切换
///
/// - Stash：调 stash_save（含 untracked）保存工作区，再 checkout；stash 不自动 pop（用户自己决定）
/// - Discard：调 discard 所有 dirty 路径丢弃工作区，再 checkout（红色危险按钮）
pub(super) fn open_checkout_dirty_dialog(
    view: Entity<VcsView>,
    target: String,
    window: &mut Window,
    cx: &mut Context<VcsView>,
) {
    let title = SharedString::from("工作区有未提交改动");
    let desc = format!(
        "切换到「{target}」会与当前未提交的改动冲突，git 会拒绝切换。\n\n\
         - 「Stash 后切换」：把当前改动暂存到 stash 列表，切换后可在 Stash 面板恢复\n\
         - 「丢弃后切换」：直接丢弃所有未暂存 / 已暂存改动（不可恢复）"
    );
    window.open_dialog(cx, move |dialog, _, _| {
        let view_cancel = view.clone();
        let view_stash = view.clone();
        let view_discard = view.clone();
        let target_stash = target.clone();
        let target_discard = target.clone();
        let desc = desc.clone();
        dialog
            .title(title.clone())
            .margin_top(px(180.0))
            .content(move |c, _, cx| {
                let muted_fg = cx.theme().muted_foreground;
                c.child(
                    div()
                        .py(px(4.0))
                        .text_sm()
                        .text_color(muted_fg)
                        .child(desc.clone()),
                )
            })
            .footer(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        Button::new("vcs-co-cancel")
                            .ghost()
                            .small()
                            .label("取消")
                            .on_click({
                                let _ = view_cancel;
                                |_: &ClickEvent, w, app| w.close_dialog(app)
                            }),
                    )
                    .child(
                        Button::new("vcs-co-discard")
                            .danger()
                            .small()
                            .label("丢弃后切换")
                            .on_click({
                                let v = view_discard.clone();
                                move |_: &ClickEvent, w, app| {
                                    let target = target_discard.clone();
                                    v.update(app, |this, cx| {
                                        this.run_checkout_with_discard(target, cx);
                                    });
                                    w.close_dialog(app);
                                }
                            }),
                    )
                    .child(
                        Button::new("vcs-co-stash")
                            .primary()
                            .small()
                            .label("Stash 后切换")
                            .on_click({
                                let v = view_stash.clone();
                                move |_: &ClickEvent, w, app| {
                                    let target = target_stash.clone();
                                    v.update(app, |this, cx| {
                                        this.run_checkout_with_stash(target, cx);
                                    });
                                    w.close_dialog(app);
                                }
                            }),
                    ),
            )
    });
}
