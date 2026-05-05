//! 仓库 storage 管理 + Clone / Init / 确认弹窗
//!
//! 拆出来让 mod.rs 不超 600 行红线。

use gpui::prelude::*;
use gpui::{Context, Window};
use ramag_domain::entities::{RepoConfig, RepoId};

use super::super::vcs_view::VcsView;
use super::open_repo_async;

impl VcsView {
    /// 保存单条 RepoConfig 到 storage（失败仅 warn，不阻塞 UI）
    pub(crate) fn save_repo_async(&self, repo: RepoConfig, cx: &mut Context<Self>) {
        let storage = self.storage.clone();
        cx.background_spawn(async move {
            if let Err(e) = storage.save_repo(&repo).await {
                tracing::warn!(error = %e, repo = %repo.name, "vcs: save_repo failed");
            }
        })
        .detach();
    }

    /// 从 storage 删除单条 RepoConfig（失败仅 warn）
    pub(crate) fn delete_repo_async(&self, id: RepoId, cx: &mut Context<Self>) {
        let storage = self.storage.clone();
        cx.background_spawn(async move {
            if let Err(e) = storage.delete_repo(&id).await {
                tracing::warn!(error = %e, repo_id = %id, "vcs: delete_repo failed");
            }
        })
        .detach();
    }

    /// 弹确认对话框：从最近列表移除仓库（不删磁盘文件）
    pub(crate) fn confirm_remove_recent_repo(
        &self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity();
        let name = self
            .recent_repos
            .iter()
            .find(|r| r.path == path)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| path.clone());
        ramag_ui::open_confirm(
            "从最近列表移除？",
            format!("确定从最近列表移除「{name}」吗？\n仅清除本地最近记录，不会删除磁盘文件。"),
            "移除",
            true,
            move |_window, app| {
                view.update(app, |this, cx| this.remove_recent_repo(path, cx));
            },
            window,
            cx,
        );
    }

    /// 异步 Clone 远程仓库到本地路径，完成后复用 open_repo_async 走 open + 拉数据流
    pub(crate) fn clone_repo_async(
        &mut self,
        url: String,
        dest: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let driver = self.driver.clone();
        self.loading = true;
        self.error = None;
        self.show_clone_panel = false;
        cx.notify();
        cx.spawn(
            async move |this, cx| match driver.clone_repo(&url, &dest).await {
                Ok(rc) => {
                    tracing::info!(url = %url, dest = ?dest, "vcs: clone done");
                    open_repo_async(&this, driver, std::path::PathBuf::from(&rc.path), cx).await;
                }
                Err(e) => {
                    tracing::error!(error = %e, "vcs: clone failed");
                    let _ = this.update(cx, |this, cx| {
                        this.loading = false;
                        this.error = Some(format!("Clone 失败: {e}"));
                        cx.notify();
                    });
                }
            },
        )
        .detach();
    }

    /// 异步初始化空仓库，完成后打开 session
    pub(crate) fn init_repo_async(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        let driver = self.driver.clone();
        self.loading = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            open_repo_async(&this, driver, path, cx).await;
        })
        .detach();
    }

    /// 启动时从 storage 加载 recent_repos（跨重启保留）
    pub(crate) fn load_recent_repos_async(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let storage = match this.update(cx, |this, _| this.storage.clone()) {
                Ok(s) => s,
                Err(_) => return,
            };
            let result = storage.list_repos().await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(list) => {
                    this.recent_repos = list;
                    cx.notify();
                }
                Err(e) => tracing::warn!(error = %e, "vcs: list_repos failed"),
            });
        })
        .detach();
    }
}
