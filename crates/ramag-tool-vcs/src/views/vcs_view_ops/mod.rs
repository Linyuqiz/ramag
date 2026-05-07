//! VcsView 异步操作：分支 / commit / 文件 op / 历史分页（stash/tag/remote 在子模块）

mod remote;
mod stash;
mod tag;

use gpui::Context;
use ramag_domain::entities::{BranchKind, LogOptions};
use tracing::{error, info};

use super::helpers::{BranchOp, FileOp, FileTabSource, HISTORY_PAGE_SIZE};
use super::vcs_view::VcsView;
use super::vcs_view_ops_history::parse_search_query;

impl VcsView {
    /// checkout / create / delete / merge / rebase
    pub(in crate::views) fn run_branch_op(&mut self, op: BranchOp, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.busy = true;
        self.error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = match &op {
                BranchOp::Checkout(name) => driver.checkout(&repo, name).await,
                BranchOp::Create(name, base) => {
                    // 等价 `git checkout -b`：创建后立即 checkout
                    let r = driver.create_branch(&repo, name, base.as_deref()).await;
                    if r.is_ok() {
                        let _ = driver.checkout(&repo, name).await;
                    }
                    r
                }
                BranchOp::Delete(name, force) => driver.delete_branch(&repo, name, *force).await,
                // --no-ff 强制建 merge commit；冲突时仓库进入 Merge 状态
                BranchOp::Merge(name) => driver.merge(&repo, name, true, false, None).await,
                BranchOp::Rebase(name) => driver.rebase(&repo, name).await,
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
                if let Err(e) = result {
                    error!(error = %e, ?op, "vcs: branch op failed");
                    this.error = Some(format!("分支操作失败：{e}"));
                } else if matches!(
                    op,
                    BranchOp::Checkout(_)
                        | BranchOp::Merge(_)
                        | BranchOp::Rebase(_)
                        | BranchOp::Create(_, _)
                ) {
                    // HEAD 变了，缓存全失效
                    this.load_history_page(0, cx);
                    this.refresh_after_head_change(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// HEAD 变化（checkout / merge / rebase / 建分支）：清缓存 + 重拉，避免显示旧分支内容
    pub(in crate::views) fn refresh_after_head_change(&mut self, cx: &mut Context<Self>) {
        for tab in &mut self.file_tabs {
            tab.cached_diff = None;
            tab.cached_content = None;
        }
        self.current_diff = None;
        self.current_file_content = None;
        self.commit_file_diff = None;
        self.blame_lines.clear();
        self.selected_diff_lines.clear();

        self.refresh_current_files_view(cx);

        if let Some(idx) = self.active_file_tab_idx
            && let Some(tab) = self.file_tabs.get(idx).cloned()
        {
            match tab.source {
                FileTabSource::Changes(kind) => {
                    self.select_file(tab.path, kind, cx);
                }
                FileTabSource::ProjectFiles => {
                    self.select_pf_file(tab.path, cx);
                }
                FileTabSource::Commit { commit_id, .. } => {
                    self.select_commit_file(tab.path, commit_id, cx);
                }
            }
        }
    }

    /// base=None 时从当前 HEAD 建
    pub(in crate::views) fn handle_create_branch(&mut self, cx: &mut Context<Self>) {
        let name = self.create_branch_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.error = Some("分支名不能为空".into());
            cx.notify();
            return;
        }
        let base = self.create_branch_base.take();
        self.run_branch_op(BranchOp::Create(name, base), cx);
    }

    pub(in crate::views) fn set_create_branch_base(
        &mut self,
        base: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.create_branch_base = base;
        cx.notify();
    }

    /// skip=0 覆盖刷新，其他值 append
    pub(in crate::views) fn load_history_page(&mut self, skip: usize, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        if self.loading_history {
            return;
        }
        self.loading_history = true;
        cx.notify();

        let driver = self.driver.clone();
        // `@xxx`→author，`7d`/`1m`→since，其余→message grep
        let raw_search = self
            .history_search_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let (grep, author, since) = parse_search_query(&raw_search);
        let opts = LogOptions {
            skip,
            limit: Some(HISTORY_PAGE_SIZE),
            path_filter: self.history_path_filter.clone(),
            grep,
            author,
            since,
            ..Default::default()
        };
        cx.spawn(async move |this, cx| {
            let result = driver.log(&repo, opts).await;
            let _ = this.update(cx, |this, cx| {
                this.loading_history = false;
                match result {
                    Ok(commits) => {
                        let got = commits.len();
                        if skip == 0 {
                            this.history_commits = commits;
                        } else {
                            this.history_commits.extend(commits);
                        }
                        this.history_has_more = got >= HISTORY_PAGE_SIZE;
                    }
                    Err(e) => {
                        error!(error = %e, "vcs: load history failed");
                        this.error = Some(format!("加载历史失败：{e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::views) fn run_commit(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let message = self.commit_input.read(cx).value().trim().to_string();
        if message.is_empty() && !self.commit_amend {
            self.error = Some("commit message 不能为空".into());
            cx.notify();
            return;
        }
        let amend = self.commit_amend;
        let sign = self.commit_sign;
        let driver = self.driver.clone();
        self.busy = true;
        self.error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = driver.commit(&repo, &message, amend, sign).await;
            let new_status = if result.is_ok() {
                driver.status(&repo).await.ok()
            } else {
                None
            };
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(commit_id) => {
                        info!(commit = %commit_id, "vcs: commit done");
                        if let Some(s) = new_status {
                            this.status = Some(s);
                        }
                        // 关闭 amend；message 保留方便再次提交
                        this.commit_amend = false;
                    }
                    Err(e) => {
                        error!(error = %e, "vcs: commit failed");
                        this.error = Some(format!("提交失败：{e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::views) fn run_file_op(
        &mut self,
        op: FileOp,
        path: String,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.busy = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let paths = vec![path.clone()];
            let result = match op {
                FileOp::Stage => driver.stage(&repo, &paths).await,
                FileOp::Unstage => driver.unstage(&repo, &paths).await,
                FileOp::Discard => driver.discard(&repo, &paths).await,
            };
            let new_status = if result.is_ok() {
                driver.status(&repo).await.ok()
            } else {
                None
            };
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(_) => {
                        if let Some(s) = new_status {
                            this.status = Some(s);
                        }
                    }
                    Err(e) => {
                        error!(error = %e, ?op, ?path, "vcs: file op failed");
                        this.error = Some(format!("操作失败：{e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
