//! VcsView 远程仓库管理 ops：list / add / remove / set_url
//!
//! 与 vcs_view_ops 的 run_remote_op（fetch/pull/push）不同——这里管理「remote 配置本身」，
//! 不是「与 remote 交互」。
//!
//! 注：sidebar panel 删除后 add/remove/set_url 暂无 UI 入口；reload_remotes 仍由 open_repo 调用。

#![allow(dead_code)]

use gpui::Context;
use tracing::{error, info};

use super::vcs_view::VcsView;

/// 远程仓库管理操作（add / remove / 修改 URL）
#[derive(Debug, Clone)]
pub(super) enum RemoteAdminOp {
    /// (name, url)
    Add { name: String, url: String },
    /// 删除指定 remote
    Remove(String),
    /// (name, new_url) 修改 fetch URL（UI 暂未暴露，预留给「编辑」对话框）
    #[allow(dead_code)]
    SetUrl { name: String, url: String },
}

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

    /// remote 管理操作派发：add / remove / set_url
    pub(super) fn run_remote_admin_op(&mut self, op: RemoteAdminOp, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.busy = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = match &op {
                RemoteAdminOp::Add { name, url } => driver.add_remote(&repo, name, url).await,
                RemoteAdminOp::Remove(name) => driver.remove_remote(&repo, name).await,
                RemoteAdminOp::SetUrl { name, url } => {
                    driver.set_remote_url(&repo, name, url).await
                }
            };
            let new_remotes = driver.list_remotes(&repo).await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.remotes = new_remotes;
                if let Err(e) = result {
                    error!(error = %e, ?op, "vcs: remote admin op failed");
                    this.error = Some(format!("Remote 管理失败：{e}"));
                } else {
                    info!(?op, "vcs: remote admin op done");
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 「添加 remote」按钮触发：读 input 框 → run_remote_admin_op
    pub(super) fn handle_add_remote(&mut self, cx: &mut Context<Self>) {
        let name = self
            .add_remote_name_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let url = self
            .add_remote_url_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if name.is_empty() || url.is_empty() {
            self.error = Some("remote 名 / URL 都不能为空".into());
            cx.notify();
            return;
        }
        self.run_remote_admin_op(RemoteAdminOp::Add { name, url }, cx);
    }
}
