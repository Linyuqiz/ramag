//! VcsView 远程仓库列表加载
//!
//! 仅保留 `reload_remotes`（open_repo 后由 vcs_view_ops_repo 调用刷新远程列表）。
//! Remote 配置写操作（add / remove / set_url）随旧 sidebar panel 一并删除。

use gpui::Context;
use tracing::error;

use super::vcs_view::VcsView;

impl VcsView {
    /// 异步加载 remote 列表
    pub(super) fn reload_remotes(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.loading_remotes = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = driver.list_remotes(&repo).await;
            let _ = this.update(cx, |this, cx| {
                this.loading_remotes = false;
                match result {
                    Ok(list) => this.remotes = list,
                    Err(e) => error!(error = %e, "vcs: list_remotes failed"),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
