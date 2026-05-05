//! VcsView reflog 相关 ops：toggle 视图 / 加载 reflog / checkout 到 reflog 条目

use gpui::Context;
use tracing::{error, info};

use super::super::vcs_view::VcsView;

impl VcsView {
    /// 切换 reflog / commit 视图
    pub(crate) fn toggle_reflog(&mut self, cx: &mut Context<Self>) {
        self.showing_reflog = !self.showing_reflog;
        if self.showing_reflog {
            self.load_reflog(cx);
        }
        cx.notify();
    }

    /// 异步拉取 reflog（默认 HEAD，最多 200 条）
    pub(crate) fn load_reflog(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.loading_reflog = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = driver.list_reflog(&repo, None, Some(200)).await;
            let _ = this.update(cx, |this, cx| {
                this.loading_reflog = false;
                match result {
                    Ok(entries) => this.reflog_entries = entries,
                    Err(e) => {
                        error!(error = %e, "vcs: list_reflog failed");
                        this.error = Some(format!("加载 reflog 失败：{e}"));
                        this.showing_reflog = false;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// reflog 条目点击 → checkout 到该 commit（detached HEAD；checkout 后切回 commit 历史）
    pub(crate) fn checkout_reflog_entry(&mut self, commit: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.busy = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = driver.checkout(&repo, &commit).await;
            let new_status = driver.status(&repo).await.ok();
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if let Some(s) = new_status {
                    this.status = Some(s);
                }
                if let Err(e) = result {
                    error!(error = %e, %commit, "vcs: reflog checkout failed");
                    this.error = Some(format!("Checkout 到 {commit} 失败：{e}"));
                } else {
                    info!(%commit, "vcs: reflog checkout done");
                    this.showing_reflog = false;
                    this.load_history_page(0, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
}
