//! VcsView Stash 异步操作：加载列表 + save / apply / pop / drop

use gpui::Context;
use tracing::error;

use super::super::helpers::StashOp;
use super::super::vcs_view::VcsView;

impl VcsView {
    /// 异步加载 stash 列表（仓库打开时 + stash 操作完成后调用）
    pub(in crate::views) fn reload_stashes(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.loading_stashes = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = driver.list_stashes(&repo).await;
            let _ = this.update(cx, |this, cx| {
                this.loading_stashes = false;
                if !this.is_current_repo(&repo) {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(list) => this.stashes = list,
                    Err(e) => {
                        error!(error = %e, "vcs: list stashes failed");
                        this.error = Some("加载 Stash 列表失败".into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 主动 stash 当前工作区改动（含 untracked）；message 为空用 git 默认描述
    pub(in crate::views) fn run_stash_save(&mut self, message: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.busy = true;
        self.busy_label = Some("Stash 中…");
        self.error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let msg = message.trim().to_string();
            let msg_opt = (!msg.is_empty()).then_some(msg.as_str());
            let result = driver.stash_save(&repo, msg_opt, true).await;
            let new_stashes = driver.list_stashes(&repo).await.unwrap_or_default();
            let new_status = driver.status(&repo).await.ok();
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.busy_label = None;
                if !this.is_current_repo(&repo) {
                    cx.notify();
                    return;
                }
                this.stashes = new_stashes;
                if let Some(s) = new_status {
                    this.status = Some(s);
                }
                match result {
                    Err(e) => {
                        error!(error = %e, "vcs: stash save failed");
                        this.error = Some(format!("Stash 失败：{e}"));
                    }
                    Ok(_) => {
                        // 工作区被清空 → 已开的 Changes tabs 全部失效
                        this.sync_changes_tabs_with_status(cx);
                        this.notify_success("已 stash 工作区改动（含未跟踪文件）", cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Stash 操作：保存 / 应用 / 弹出 / 删除
    pub(in crate::views) fn run_stash_op(&mut self, op: StashOp, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.busy = true;
        self.error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = match op {
                StashOp::Apply(idx) => driver.stash_apply(&repo, idx, false).await,
                StashOp::Pop(idx) => driver.stash_apply(&repo, idx, true).await,
                StashOp::Drop(idx) => driver.stash_drop(&repo, idx).await,
            };
            // 操作后刷新 stashes + status
            let new_stashes = driver.list_stashes(&repo).await.unwrap_or_default();
            let new_status = driver.status(&repo).await.ok();
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.busy_label = None;
                if !this.is_current_repo(&repo) {
                    cx.notify();
                    return;
                }
                this.stashes = new_stashes;
                if let Some(s) = new_status {
                    this.status = Some(s);
                }
                match result {
                    Err(e) => {
                        error!(error = %e, ?op, "vcs: stash op failed");
                        this.error = Some(format!("Stash 操作失败：{e}"));
                    }
                    Ok(_) => {
                        // apply / pop 会改工作区文件 → tabs 对齐
                        this.sync_changes_tabs_with_status(cx);
                        let msg = match op {
                            StashOp::Apply(_) => "已应用 stash（保留堆栈条目）",
                            StashOp::Pop(_) => "已弹出 stash 到工作区",
                            StashOp::Drop(_) => "已删除 stash",
                        };
                        this.notify_success(msg, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
