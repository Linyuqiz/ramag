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
        self.busy_label = Some(match &op {
            BranchOp::Checkout(_) => "切换分支中…",
            BranchOp::Create(_, _) => "创建分支中…",
            BranchOp::Delete(_, _) => "删除分支中…",
            BranchOp::Merge(_) => "合并中…",
            BranchOp::Rebase(_) => "Rebase 中…",
        });
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
                this.busy_label = None;
                if !this.is_current_repo(&repo) {
                    cx.notify();
                    return;
                }
                this.local_branches = new_local;
                if let Some(s) = new_status {
                    this.status = Some(s);
                }
                match &result {
                    Err(e) => {
                        error!(error = %e, ?op, "vcs: branch op failed");
                        this.error = Some(format!("分支操作失败：{e}"));
                    }
                    Ok(_) => {
                        let done_msg = match &op {
                            BranchOp::Checkout(n) => format!("已切换到 {n}"),
                            BranchOp::Create(n, _) => format!("已创建并切换到 {n}"),
                            BranchOp::Delete(n, _) => format!("已删除分支 {n}"),
                            BranchOp::Merge(n) => format!("已合并 {n}"),
                            BranchOp::Rebase(n) => format!("已 rebase 到 {n}"),
                        };
                        this.notify_success(done_msg, cx);
                    }
                }
                if result.is_ok()
                    && matches!(
                        op,
                        BranchOp::Checkout(_)
                            | BranchOp::Merge(_)
                            | BranchOp::Rebase(_)
                            | BranchOp::Create(_, _)
                    )
                {
                    // HEAD 变了，缓存全失效
                    this.load_history_page(0, cx);
                    this.refresh_after_head_change(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// HEAD 变化（checkout / merge / rebase / 建分支 / pull）：清缓存 + 重拉，避免显示旧分支内容
    pub(in crate::views) fn refresh_after_head_change(&mut self, cx: &mut Context<Self>) {
        for tab in &mut self.file_tabs {
            tab.cached_diff = None;
            tab.cached_content = None;
        }
        self.current_diff = None;
        self.current_file_content = None;
        self.commit_file_diff = None;
        self.blame_lines.clear();

        self.refresh_current_files_view(cx);
        // Changes tabs 对齐新 status（关已无变更的 / 重定向组别），active 是 Changes 时由它重拉
        self.sync_changes_tabs_with_status(cx);

        if let Some(idx) = self.active_file_tab_idx
            && let Some(tab) = self.file_tabs.get(idx).cloned()
        {
            match tab.source {
                // Changes 来源已由 sync_changes_tabs_with_status 重拉
                FileTabSource::Changes(_) => {}
                FileTabSource::ProjectFiles => {
                    self.select_pf_file(tab.path, cx);
                }
                FileTabSource::Commit { commit_id, .. } => {
                    self.select_commit_file(tab.path, commit_id, cx);
                }
            }
        }
    }

    /// 切换 amend：勾上且 message 为空时，异步拉 HEAD 的 message 填入输入框（IDEA 同款），
    /// 方便在原文基础上改；取消勾选不动已输入内容
    pub(in crate::views) fn toggle_commit_amend(&mut self, cx: &mut Context<Self>) {
        self.commit_amend = !self.commit_amend;
        cx.notify();
        if !self.commit_amend {
            return;
        }
        let input_empty = self.commit_input.read(cx).value().trim().is_empty();
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        if !input_empty {
            return;
        }
        let driver = self.driver.clone();
        cx.spawn(async move |this, cx| {
            let opts = LogOptions {
                limit: Some(1),
                ..Default::default()
            };
            let head_msg = driver
                .log(&repo, opts)
                .await
                .ok()
                .and_then(|commits| commits.first().map(|c| c.message_full()));
            let _ = this.update(cx, |this, cx| {
                if !this.is_current_repo(&repo) || !this.commit_amend {
                    return;
                }
                // 异步期间用户已输入内容则不覆盖
                if let Some(msg) = head_msg
                    && this.commit_input.read(cx).value().trim().is_empty()
                {
                    this.pending_commit_text = Some(msg.into());
                    cx.notify();
                }
            });
        })
        .detach();
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
        // skip>0 是 load-more：正在加载时跳过避免重复拉同一页；
        // skip=0 是刷新/切仓/换搜索，即使有在途请求也要发起（否则切仓后新仓库 history 会因早退而不加载）
        if skip > 0 && self.loading_history {
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
                if !this.is_current_repo(&repo) {
                    cx.notify();
                    return;
                }
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
        self.busy_label = Some("提交中…");
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
                this.busy_label = None;
                if !this.is_current_repo(&repo) {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(commit_id) => {
                        info!(commit = %commit_id, "vcs: commit done");
                        if let Some(s) = new_status {
                            this.status = Some(s);
                        }
                        this.commit_amend = false;
                        // 提交成功：清空 message（避免下次误用同一条），已提交文件的 tabs 对齐
                        this.pending_commit_text = Some(gpui::SharedString::default());
                        this.sync_changes_tabs_with_status(cx);
                        // history 已加载过 / 面板开着 → 立即把新 commit 刷到列表顶部
                        if this.history_pane_visible || !this.history_commits.is_empty() {
                            this.load_history_page(0, cx);
                        }
                        let short: String = commit_id.0.chars().take(7).collect();
                        this.notify_success(format!("已提交 {short}"), cx);
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

    /// stage / unstage / discard，支持多文件批量（「全部 Stage」一次任务搞定，只刷一次 status）
    pub(in crate::views) fn run_file_op(
        &mut self,
        op: FileOp,
        paths: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        if paths.is_empty() {
            return;
        }
        let driver = self.driver.clone();
        self.busy = true;
        cx.notify();

        cx.spawn(async move |this, cx| {
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
                this.busy_label = None;
                if !this.is_current_repo(&repo) {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(_) => {
                        if let Some(s) = new_status {
                            this.status = Some(s);
                        }
                        // 组别迁移（如 stage 后 Unstaged → Staged）跟着对齐
                        this.sync_changes_tabs_with_status(cx);
                        if matches!(op, FileOp::Discard) {
                            let target = if paths.len() == 1 {
                                paths[0].clone()
                            } else {
                                format!("{} 个文件", paths.len())
                            };
                            this.notify_success(format!("已丢弃 {target} 的改动"), cx);
                        }
                    }
                    Err(e) => {
                        error!(error = %e, ?op, ?paths, "vcs: file op failed");
                        this.error = Some(format!("操作失败：{e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
