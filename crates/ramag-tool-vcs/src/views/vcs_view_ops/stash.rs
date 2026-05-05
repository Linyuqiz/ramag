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
                match result {
                    Ok(list) => this.stashes = list,
                    Err(e) => {
                        error!(error = %e, "vcs: list stashes failed");
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
                this.stashes = new_stashes;
                if let Some(s) = new_status {
                    this.status = Some(s);
                }
                if let Err(e) = result {
                    error!(error = %e, ?op, "vcs: stash op failed");
                    this.error = Some(format!("Stash 操作失败：{e}"));
                }
                cx.notify();
            });
        })
        .detach();
    }
}
