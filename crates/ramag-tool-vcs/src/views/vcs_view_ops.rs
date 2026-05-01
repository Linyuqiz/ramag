//! VcsView 异步操作集合
//!
//! 拆分自原 `vcs_view.rs`（>790 行），按职责把所有 `cx.spawn` 异步流抽到本 mod，
//! 让 `vcs_view.rs` 仅保留状态结构 + 入口装配 + Render。
//!
//! 包含：分支 / stash / 远程 / commit / 文件 op / 历史分页 / 文件选择 / 仓库打开。
//! 与 `vcs_view.rs` 同属 `views::vcs_view::VcsView`，因此可直接 `impl VcsView`。

use gpui::Context;
use ramag_domain::entities::{BranchKind, LogOptions};
use tracing::{error, info};

use super::helpers::{
    BranchOp, FileOp, FileTabSource, HISTORY_PAGE_SIZE, RemoteOp, StashOp, TagOp,
};
use super::vcs_view::VcsView;
use super::vcs_view_ops_history::parse_search_query;

impl VcsView {
    /// 分支操作：checkout / create / delete
    pub(super) fn run_branch_op(&mut self, op: BranchOp, cx: &mut Context<Self>) {
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
                    // 创建后自动 checkout 到新分支（git checkout -b 行为）
                    let r = driver.create_branch(&repo, name, base.as_deref()).await;
                    if r.is_ok() {
                        let _ = driver.checkout(&repo, name).await;
                    }
                    r
                }
                BranchOp::Delete(name, force) => driver.delete_branch(&repo, name, *force).await,
                // --no-ff：默认强制建 merge commit；冲突时操作返回 Err 但仓库已进入 Merge 状态
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
                    // HEAD 变化：history 列表跟着切换；Project Files / 当前 diff /
                    // 文件内容缓存全部失效，重新拉一次
                    this.load_history_page(0, cx);
                    this.refresh_after_head_change(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// HEAD 变化（checkout / merge / rebase / 创建分支）后清缓存 + 重拉数据
    ///
    /// 切分支后：Project Files 列表、当前文件 diff / 内容、blame、commit detail diff
    /// 都属于「分支特定」，必须重读；不重读会让用户看到旧分支的内容（最常见的"切了没刷新"投诉）
    pub(super) fn refresh_after_head_change(&mut self, cx: &mut Context<Self>) {
        // 1. 文件 tab 缓存全失效（每个 tab 在新分支上内容不一样）
        for tab in &mut self.file_tabs {
            tab.cached_diff = None;
            tab.cached_content = None;
        }
        // 2. 当前看的 diff / 文件内容 / blame / commit 详情 diff 都清掉
        self.current_diff = None;
        self.current_file_content = None;
        self.commit_file_diff = None;
        self.blame_lines.clear();
        self.selected_diff_lines.clear();

        // 3. 刷新 Project Files / Changes 列表
        self.refresh_current_files_view(cx);

        // 4. 当前若有激活 tab，按其类型重新触发加载
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

    /// 「新建分支」按钮触发：读 input 框名字 + create_branch_base 作为 base
    /// （base=None 时 BranchOp::Create 内部默认从当前 HEAD）
    pub(super) fn handle_create_branch(&mut self, cx: &mut Context<Self>) {
        let name = self.create_branch_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.error = Some("分支名不能为空".into());
            cx.notify();
            return;
        }
        let base = self.create_branch_base.take();
        self.run_branch_op(BranchOp::Create(name, base), cx);
    }

    /// 设置新建分支的 base（dropdown 内选中分支时调）
    pub(super) fn set_create_branch_base(&mut self, base: Option<String>, cx: &mut Context<Self>) {
        self.create_branch_base = base;
        cx.notify();
    }

    /// 异步加载 stash 列表（仓库打开时 + stash 操作完成后调用）
    pub(super) fn reload_stashes(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn run_stash_op(&mut self, op: StashOp, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.busy = true;
        self.error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = match op {
                StashOp::Save => driver.stash_save(&repo, None, true).await,
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

    /// 异步加载某页 commit；skip=0 等于刷新（覆盖现有），其他 skip 值 append
    pub(super) fn load_history_page(&mut self, skip: usize, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        if self.loading_history {
            return;
        }
        self.loading_history = true;
        cx.notify();

        let driver = self.driver.clone();
        // 解析搜索框：「@xxx」→ author 过滤；「7d」/「1m」→ since；纯文本 → message grep
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

    /// 用户点击 Changes 文件行：选中 + 异步拉 diff
    /// 远程同步：fetch / pull / push
    ///
    /// remote 解析策略：
    /// - 当前分支有 upstream（如 "origin/main"）→ 拆出 remote 名 + 远端分支名
    /// - 当前分支无 upstream → push 自动加 -u 设置；pull 报错引导用户先 push
    /// - fetch 总是 `git fetch --all --prune`，多 remote 一并拉
    pub(super) fn run_remote_op(&mut self, op: RemoteOp, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let Some(local_branch) = self.status.as_ref().and_then(|s| s.head_branch.clone()) else {
            self.error = Some("当前为 detached HEAD，无法 push/pull".into());
            cx.notify();
            return;
        };
        // 从 local_branches 找当前 head 的 upstream（"origin/main"）
        let upstream = self
            .local_branches
            .iter()
            .find(|b| b.is_head)
            .and_then(|b| b.upstream.clone());
        let (remote_name, remote_branch) = match upstream.as_deref().and_then(|u| u.split_once('/'))
        {
            Some((r, b)) => (r.to_string(), b.to_string()),
            None => ("origin".to_string(), local_branch.clone()),
        };
        let need_set_upstream = upstream.is_none();
        // PushForce → 走 --force-with-lease；其他 op 忽略
        let this_force_lease = matches!(op, RemoteOp::PushForce);
        // pull 模式下若没有 upstream 直接报错引导（避免提示「fatal: no tracking info」）
        if matches!(op, RemoteOp::Pull) && need_set_upstream {
            self.error =
                Some("当前分支没有上游分支：先点 Push（会自动设置 upstream）再 Pull".into());
            cx.notify();
            return;
        }
        let driver = self.driver.clone();
        self.busy = true;
        self.error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = match op {
                // 空 remote 让 driver 拉所有 remote
                RemoteOp::Fetch => driver.fetch(&repo, "").await,
                RemoteOp::Pull => {
                    driver
                        .pull(&repo, &remote_name, &remote_branch, false)
                        .await
                }
                RemoteOp::Push | RemoteOp::PushForce => {
                    driver
                        .push(
                            &repo,
                            &remote_name,
                            &local_branch,
                            need_set_upstream,
                            this_force_lease,
                        )
                        .await
                }
            };
            // 不论成功失败都刷新一次 status（pull 后 ahead/behind 必变）
            let new_status = driver.status(&repo).await.ok();
            let local = driver.list_branches(&repo, BranchKind::Local).await.ok();
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if let Err(e) = result {
                    error!(error = %e, ?op, "vcs: remote op failed");
                    this.error = Some(format!("{op:?} 失败：{e}"));
                } else {
                    info!(?op, "vcs: remote op done");
                }
                if let Some(s) = new_status {
                    this.status = Some(s);
                }
                if let Some(b) = local {
                    this.local_branches = b;
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 异步执行 commit；成功后清空 message + 刷新 status
    pub(super) fn run_commit(&mut self, cx: &mut Context<Self>) {
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
                        // 提交完关掉 amend；message 留着方便用户改完再次提交
                        // （清空 InputState 需要 window 上下文，简化先跳过）
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

    /// 异步执行 stage / unstage / discard 后刷新 status
    pub(super) fn run_file_op(&mut self, op: FileOp, path: String, cx: &mut Context<Self>) {
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

    /// 异步加载 tag 列表（仓库打开时 + tag 操作完成后调用）
    pub(super) fn reload_tags(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.loading_tags = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = driver.list_tags(&repo).await;
            let _ = this.update(cx, |this, cx| {
                this.loading_tags = false;
                match result {
                    Ok(list) => this.tags = list,
                    Err(e) => error!(error = %e, "vcs: list tags failed"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Tag 操作：创建 / 删除 / 推送
    pub(super) fn run_tag_op(&mut self, op: TagOp, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.busy = true;
        self.error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = match &op {
                TagOp::Create { name, message } => {
                    // sign 暂未在 UI 暴露——release tag 通常不签名；future 加 toggle
                    driver
                        .create_tag(&repo, name, None, message.as_deref(), false)
                        .await
                }
                TagOp::Delete(name) => driver.delete_tag(&repo, name).await,
                TagOp::Push(name) => driver.push_tag(&repo, "origin", name).await,
            };
            let new_tags = driver.list_tags(&repo).await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.tags = new_tags;
                if let Err(e) = result {
                    error!(error = %e, ?op, "vcs: tag op failed");
                    this.error = Some(format!("Tag 操作失败：{e}"));
                } else {
                    info!(?op, "vcs: tag op done");
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 「新建 tag」按钮：读 input → 调 run_tag_op
    ///
    /// message 非空时走 annotated tag；空则走 lightweight。
    pub(super) fn handle_create_tag(&mut self, cx: &mut Context<Self>) {
        let name = self.create_tag_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.error = Some("tag 名不能为空".into());
            cx.notify();
            return;
        }
        let msg_raw = self
            .create_tag_message_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let message = if msg_raw.is_empty() {
            None
        } else {
            Some(msg_raw)
        };
        self.run_tag_op(TagOp::Create { name, message }, cx);
    }
}
