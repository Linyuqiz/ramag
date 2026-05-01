//! VcsView 历史相关 ops：Reset / Revert / Commit 详情加载（HEAD 移动后自动重置 history 首页）

use gpui::{Context, SharedString};
use ramag_domain::entities::{BranchKind, DiffKind, ResetKind};
use tracing::{error, info};

use super::helpers::{BranchOp, ViewMode};
use super::vcs_view::VcsView;

impl VcsView {
    /// 进入「单文件历史」：设置 path_filter + 强制打开下半 history pane + 重新拉首页
    ///
    /// IDE 布局下 ViewMode::History 已不再用于切视图，但保留赋值兼容旧路径；
    /// 关键是把 history_pane_visible=true，否则用户点按钮看不到任何反馈
    pub(super) fn view_file_history(&mut self, path: String, cx: &mut Context<Self>) {
        self.history_path_filter = Some(path);
        self.view_mode = ViewMode::History;
        self.history_pane_visible = true;
        self.history_commits.clear();
        self.load_history_page(0, cx);
    }

    /// 清除单文件历史过滤，回到全仓库 history
    pub(super) fn clear_history_path_filter(&mut self, cx: &mut Context<Self>) {
        self.history_path_filter = None;
        self.history_commits.clear();
        self.load_history_page(0, cx);
    }

    /// 触发 commit 搜索：解析 search_input + 重新拉首页
    pub(super) fn apply_history_search(&mut self, cx: &mut Context<Self>) {
        self.history_commits.clear();
        self.load_history_page(0, cx);
    }

    /// Revert：生成一个反向 commit 撤销指定 commit
    pub(super) fn run_revert(&mut self, commit_id: String, cx: &mut Context<Self>) {
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
    pub(super) fn run_reset(&mut self, target: String, kind: ResetKind, cx: &mut Context<Self>) {
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

    /// 进入「commit 详情视图」：拉文件列表 + 自动选第一个文件 → 拉 diff
    pub(super) fn load_commit_detail(&mut self, commit_id: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        let commit = self
            .history_commits
            .iter()
            .find(|c| c.id.0 == commit_id)
            .cloned();
        self.viewing_commit = commit;
        self.commit_files = Vec::new();
        self.commit_files_collapsed.clear();
        self.selected_commit_file = None;
        self.commit_file_diff = None;
        self.loading_commit_files = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = driver.list_commit_files(&repo, &commit_id).await;
            let _ = this.update(cx, |this, cx| {
                this.loading_commit_files = false;
                match result {
                    Ok(files) => {
                        // 默认不选中任何文件，等用户主动点击
                        this.commit_files = files;
                    }
                    Err(e) => {
                        error!(error = %e, %commit_id, "vcs: list_commit_files failed");
                        this.error = Some(format!("加载 commit 文件列表失败：{e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 点选 commit 文件 → 创建 / 复用 file_tab + 拉 commit-vs-parent diff，主区与 Changes 统一
    pub(super) fn select_commit_file(
        &mut self,
        path: String,
        commit_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let change_kind = self
            .commit_files
            .iter()
            .find(|f| f.path == path)
            .and_then(|f| f.staged);
        let source = super::helpers::FileTabSource::Commit {
            commit_id: commit_id.clone(),
            change_kind,
        };
        let existing = self
            .file_tabs
            .iter()
            .position(|t| t.path == path && t.source == source);
        let idx = existing.unwrap_or_else(|| {
            self.file_tabs.push(super::helpers::FileTab {
                path: path.clone(),
                source: source.clone(),
                cached_diff: None,
                cached_content: None,
            });
            self.file_tabs.len() - 1
        });
        self.active_file_tab_idx = Some(idx);
        self.selected_commit_file = Some(path.clone());
        self.selected_file = None;
        self.selected_pf_path = None;
        self.current_file_content = None;
        // 切换 commit 文件 → 清 spacer 展开态（hunk_idx 跨文件无复用价值）
        self.expanded_diff_spacers.clear();
        if let Some(cached) = self.file_tabs[idx].cached_diff.clone() {
            self.current_diff = Some(cached.clone());
            self.commit_file_diff = Some(cached);
            self.loading_diff = false;
            cx.notify();
            return;
        }
        self.current_diff = None;
        self.commit_file_diff = None;
        self.loading_diff = true;
        cx.notify();

        let driver = self.driver.clone();
        let ignore_ws = self.diff_ignore_whitespace;
        let context_lines = self.diff_view_mode.context_lines();
        let path_for_diff = path.clone();
        cx.spawn(async move |this, cx| {
            let result = driver
                .diff_file_full_opts(
                    &repo,
                    &path_for_diff,
                    DiffKind::CommitVsParent(ramag_domain::entities::CommitId(commit_id)),
                    ignore_ws,
                    context_lines,
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading_diff = false;
                match result {
                    Ok(d) => {
                        this.current_diff = Some(d.clone());
                        this.commit_file_diff = Some(d.clone());
                        if let Some(idx) = this.active_file_tab_idx
                            && let Some(tab) = this.file_tabs.get_mut(idx)
                            && tab.path == path_for_diff
                        {
                            tab.cached_diff = Some(d);
                        }
                    }
                    Err(e) => {
                        error!(error = %e, path = %path_for_diff, "vcs: commit diff failed");
                        this.error = Some(format!("拉取 commit diff 失败：{e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 退出 commit 详情视图，回到 history 列表
    pub(super) fn close_commit_detail(&mut self, cx: &mut Context<Self>) {
        self.viewing_commit = None;
        self.commit_files = Vec::new();
        self.commit_files_collapsed.clear();
        self.selected_commit_file = None;
        self.commit_file_diff = None;
        cx.notify();
    }

    /// 工作区 dirty 切换分支：stash → checkout（stash 不自动 pop，用户在 Stash 面板手动 apply）
    pub(super) fn run_checkout_with_stash(&mut self, target: String, cx: &mut Context<Self>) {
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
    pub(super) fn run_checkout_with_discard(&mut self, target: String, cx: &mut Context<Self>) {
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

    /// 行号点击 → 拉当前文件 blame，命中行写到顶部 banner（_is_old 占位，预留 HEAD 侧逻辑）
    pub(super) fn show_inline_blame(
        &mut self,
        line_no: u32,
        _is_old: bool,
        cx: &mut Context<Self>,
    ) {
        let path = self
            .selected_file
            .as_ref()
            .map(|(p, _)| p.clone())
            .or_else(|| self.selected_commit_file.clone())
            .or_else(|| self.selected_pf_path.clone());
        let Some(path) = path else {
            return;
        };
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        if self.inline_blame_text.as_deref() == Some("加载行作者信息...") {
            return;
        }
        self.inline_blame_text = Some("加载行作者信息...".into());
        cx.notify();
        let driver = self.driver.clone();
        cx.spawn(async move |this, cx| {
            let result = driver.blame(&repo, &path).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(lines) => {
                        if let Some(b) = lines.iter().find(|l| l.line_no == line_no) {
                            let short = b.commit.0.chars().take(7).collect::<String>();
                            let date = b.timestamp.format("%Y-%m-%d");
                            this.inline_blame_text = Some(SharedString::from(format!(
                                "L{line_no}　{short}　·　{}　·　{date}　·　{}",
                                b.author, b.subject
                            )));
                        } else {
                            this.inline_blame_text =
                                Some(SharedString::from(format!("L{line_no}：未找到 blame 信息")));
                        }
                    }
                    Err(e) => {
                        error!(error = %e, %path, "vcs: inline blame failed");
                        this.inline_blame_text =
                            Some(SharedString::from(format!("blame 失败：{e}")));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 清空 inline blame banner（用户切文件 / 关闭按钮 / 切视图时调）
    pub(super) fn clear_inline_blame(&mut self, cx: &mut Context<Self>) {
        if self.inline_blame_text.is_some() {
            self.inline_blame_text = None;
            cx.notify();
        }
    }

    /// 切换 diff/blame 视图；showing_blame=true 拉 blame，否则清空
    /// 路径优先取 selected_file（Changes），其次 selected_commit_file（commit tab）
    pub(super) fn toggle_blame(&mut self, cx: &mut Context<Self>) {
        self.showing_blame = !self.showing_blame;
        if self.showing_blame {
            let path = self
                .selected_file
                .as_ref()
                .map(|(p, _)| p.clone())
                .or_else(|| self.selected_commit_file.clone());
            if let Some(p) = path {
                self.load_blame(p, cx);
            } else {
                self.showing_blame = false;
            }
        } else {
            self.blame_lines.clear();
        }
        cx.notify();
    }

    /// 切换 reflog / commit 视图
    pub(super) fn toggle_reflog(&mut self, cx: &mut Context<Self>) {
        self.showing_reflog = !self.showing_reflog;
        if self.showing_reflog {
            self.load_reflog(cx);
        }
        cx.notify();
    }

    /// 异步拉取 reflog（默认 HEAD，最多 200 条）
    pub(super) fn load_reflog(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn checkout_reflog_entry(&mut self, commit: String, cx: &mut Context<Self>) {
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

    /// 异步拉取指定文件的 blame
    pub(super) fn load_blame(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.loading_blame = true;
        self.blame_lines = Vec::new();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = driver.blame(&repo, &path).await;
            let _ = this.update(cx, |this, cx| {
                this.loading_blame = false;
                match result {
                    Ok(lines) => this.blame_lines = lines,
                    Err(e) => {
                        error!(error = %e, %path, "vcs: blame failed");
                        this.error = Some(format!("Blame 失败：{e}"));
                        this.showing_blame = false;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// 解析搜索框 → (grep, author, since)：`@xxx`=author / `7d|1w|2m|12h|3y`=since / 其他=grep
pub(super) fn parse_search_query(q: &str) -> (Option<String>, Option<String>, Option<String>) {
    if q.is_empty() {
        return (None, None, None);
    }
    let mut grep_parts: Vec<String> = Vec::new();
    let mut author: Option<String> = None;
    let mut since: Option<String> = None;
    for tok in q.split_whitespace() {
        if let Some(name) = tok.strip_prefix('@')
            && !name.is_empty()
        {
            author = Some(name.to_string());
            continue;
        }
        if let Some(s) = parse_relative_time(tok) {
            since = Some(s);
            continue;
        }
        grep_parts.push(tok.to_string());
    }
    let grep = if grep_parts.is_empty() {
        None
    } else {
        Some(grep_parts.join(" "))
    };
    (grep, author, since)
}

/// 把 `7d` / `1w` / `2m` / `12h` / `3y` 转成 git --since 接受的字符串
fn parse_relative_time(s: &str) -> Option<String> {
    if s.len() < 2 {
        return None;
    }
    let (num_part, unit) = s.split_at(s.len() - 1);
    let n: u32 = num_part.parse().ok()?;
    let unit_word = match unit {
        "h" => "hours",
        "d" => "days",
        "w" => "weeks",
        "m" => "months",
        "y" => "years",
        _ => return None,
    };
    Some(format!("{n} {unit_word} ago"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_pure_keyword_into_grep() {
        let (g, a, s) = parse_search_query("bug fix");
        assert_eq!(g.as_deref(), Some("bug fix"));
        assert!(a.is_none());
        assert!(s.is_none());
    }

    #[test]
    fn parses_author_prefix() {
        let (g, a, s) = parse_search_query("@alice");
        assert!(g.is_none());
        assert_eq!(a.as_deref(), Some("alice"));
        assert!(s.is_none());
    }

    #[test]
    fn parses_relative_time() {
        let (g, a, s) = parse_search_query("7d");
        assert!(g.is_none());
        assert!(a.is_none());
        assert_eq!(s.as_deref(), Some("7 days ago"));
    }

    #[test]
    fn mixes_three_kinds() {
        let (g, a, s) = parse_search_query("bug @alice 1w");
        assert_eq!(g.as_deref(), Some("bug"));
        assert_eq!(a.as_deref(), Some("alice"));
        assert_eq!(s.as_deref(), Some("1 weeks ago"));
    }
}
