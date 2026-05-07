//! VcsView 历史 ops：history 切换 + commit 详情 + 搜索解析。子模块 modify / blame_ops / reflog_ops

mod blame_ops;
mod modify;
mod reflog_ops;
#[cfg(test)]
mod tests;

use gpui::Context;
use ramag_domain::entities::DiffKind;
use tracing::error;

use super::helpers::ViewMode;
use super::vcs_view::VcsView;

impl VcsView {
    /// 单文件历史：设 path_filter + 打开下半 history pane（history_pane_visible=true 必需，否则无反馈）
    pub(crate) fn view_file_history(&mut self, path: String, cx: &mut Context<Self>) {
        self.history_path_filter = Some(path);
        self.view_mode = ViewMode::History;
        self.history_pane_visible = true;
        self.history_commits.clear();
        self.load_history_page(0, cx);
    }

    /// 清除单文件历史过滤，回到全仓库 history
    pub(crate) fn clear_history_path_filter(&mut self, cx: &mut Context<Self>) {
        self.history_path_filter = None;
        self.history_commits.clear();
        self.load_history_page(0, cx);
    }

    /// 触发 commit 搜索：解析 search_input + 重新拉首页
    pub(crate) fn apply_history_search(&mut self, cx: &mut Context<Self>) {
        self.history_commits.clear();
        self.load_history_page(0, cx);
    }

    /// 进入「commit 详情视图」：拉文件列表 + 自动选第一个文件 → 拉 diff
    pub(crate) fn load_commit_detail(&mut self, commit_id: String, cx: &mut Context<Self>) {
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
    pub(crate) fn select_commit_file(
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
    pub(crate) fn close_commit_detail(&mut self, cx: &mut Context<Self>) {
        self.viewing_commit = None;
        self.commit_files = Vec::new();
        self.commit_files_collapsed.clear();
        self.selected_commit_file = None;
        self.commit_file_diff = None;
        cx.notify();
    }
}

/// 解析搜索框 → (grep, author, since)：`@xxx`=author / `7d|1w|2m|12h|3y`=since / 其他=grep
pub(crate) fn parse_search_query(q: &str) -> (Option<String>, Option<String>, Option<String>) {
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
pub(crate) fn parse_relative_time(s: &str) -> Option<String> {
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
