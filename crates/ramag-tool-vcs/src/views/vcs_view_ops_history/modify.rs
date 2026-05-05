//! VcsView 历史变更（破坏性 / HEAD 移动）：Reset / Revert / 切换分支前的 stash / discard

use gpui::Context;
use ramag_domain::entities::{BranchKind, ResetKind};
use tracing::{error, info};

use super::super::helpers::BranchOp;
use super::super::vcs_view::VcsView;

impl VcsView {
    /// Revert：生成一个反向 commit 撤销指定 commit
    pub(crate) fn run_revert(&mut self, commit_id: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.busy = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = driver.revert(&repo, &commit_id).await;
            let new_status = driver.status(&repo).await.ok();
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if let Some(s) = new_status {
                    this.status = Some(s);
                }
                if let Err(e) = result {
                    error!(error = %e, %commit_id, "vcs: revert failed");
                    this.error = Some(format!("Revert 失败：{e}（如有冲突请到工作区处理）"));
                } else {
                    info!(%commit_id, "vcs: revert done");
                    // HEAD 推进一个 revert commit，刷新 history 第一页
                    this.load_history_page(0, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Reset：移动 HEAD 到指定 commit（默认 mixed，hard 留弹框确认避免误操作）
    pub(crate) fn run_reset(&mut self, target: String, kind: ResetKind, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.busy = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = driver.reset(&repo, &target, kind).await;
            let new_status = driver.status(&repo).await.ok();
            let new_local = driver
                .list_branches(&repo, BranchKind::Local)
                .await
                .unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if let Some(s) = new_status {
                    this.status = Some(s);
                }
                this.local_branches = new_local;
                if let Err(e) = result {
                    error!(error = %e, %target, ?kind, "vcs: reset failed");
                    this.error = Some(format!("Reset {kind:?} 失败：{e}"));
                } else {
                    info!(%target, ?kind, "vcs: reset done");
                    // HEAD 移动了，history 列表与暂存区状态都要重拉
                    this.load_history_page(0, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 工作区 dirty 切换分支：stash → checkout（stash 不自动 pop，用户在 Stash 面板手动 apply）
    pub(crate) fn run_checkout_with_stash(&mut self, target: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.busy = true;
        self.error = None;
        cx.notify();
        let target_for_log = target.clone();
        cx.spawn(async move |this, cx| {
            let msg = format!("auto-stash before checkout to {target_for_log}");
            let stash_result = driver.stash_save(&repo, Some(&msg), true).await;
            let final_result = match stash_result {
                Ok(()) => driver.checkout(&repo, &target).await,
                Err(e) => Err(e),
            };
            let new_status = driver.status(&repo).await.ok();
            let new_local = driver
                .list_branches(&repo, BranchKind::Local)
                .await
                .unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.local_branches = new_local;
                if let Some(s) = new_status {
                    this.status = Some(s);
                }
                match final_result {
                    Ok(()) => {
                        info!(target = %target_for_log, "vcs: stash + checkout done");
                        this.load_history_page(0, cx);
                        this.refresh_after_head_change(cx);
                        this.reload_stashes(cx);
                    }
                    Err(e) => {
                        error!(error = %e, target = %target_for_log, "vcs: stash+checkout failed");
                        this.error = Some(format!("Stash 后切换失败：{e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 工作区 dirty 时切换分支：discard 所有 dirty 路径 → checkout（不可逆，调用前已确认）
    pub(crate) fn run_checkout_with_discard(&mut self, target: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let Some(status) = self.status.as_ref() else {
            return;
        };
        let paths: Vec<String> = status
            .files
            .iter()
            .filter(|f| f.staged.is_some() || f.unstaged.is_some())
            .map(|f| f.path.clone())
            .collect();
        if paths.is_empty() {
            self.run_branch_op(BranchOp::Checkout(target), cx);
            return;
        }
        let driver = self.driver.clone();
        self.busy = true;
        self.error = None;
        cx.notify();
        let target_for_log = target.clone();
        cx.spawn(async move |this, cx| {
            let discard_result = driver.discard(&repo, &paths).await;
            let final_result = match discard_result {
                Ok(()) => driver.checkout(&repo, &target).await,
                Err(e) => Err(e),
            };
            let new_status = driver.status(&repo).await.ok();
            let new_local = driver
                .list_branches(&repo, BranchKind::Local)
                .await
                .unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.local_branches = new_local;
                if let Some(s) = new_status {
                    this.status = Some(s);
                }
                match final_result {
                    Ok(()) => {
                        info!(target = %target_for_log, "vcs: discard + checkout done");
                        this.load_history_page(0, cx);
                        this.refresh_after_head_change(cx);
                    }
                    Err(e) => {
                        error!(error = %e, target = %target_for_log, "vcs: discard+checkout failed");
                        this.error = Some(format!("丢弃后切换失败：{e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
