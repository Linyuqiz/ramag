//! 文件 tab 管理：select_file / close_file_tab / activate_file_tab_state
//!
//! 拆分自 `vcs_view_ops.rs`，避免单文件超 600 行硬上限。
//! 与 `vcs_view_ops.rs` 同属 `views::vcs_view::VcsView`，因此可直接 `impl VcsView`。

use gpui::Context;
use ramag_domain::entities::DiffKind;
use tracing::error;

use super::helpers::{FileTab, FileTabSource, GroupKind};
use super::vcs_view::VcsView;

impl VcsView {
    /// 选中文件查看 diff（Changes 模式）：tab 已存在则复用并优先展示缓存；否则新开 tab + 异步拉
    pub(super) fn select_file(&mut self, path: String, kind: GroupKind, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        // 切换文件 → 清掉 spacer 展开态（hunk_idx 随 diff 变化，跨文件保留无意义）
        self.expanded_diff_spacers.clear();
        // 点击 Changes 文件 → 关掉 commit detail，避免主区残留 commit diff
        if self.viewing_commit.is_some() {
            self.viewing_commit = None;
            self.commit_files.clear();
            self.commit_files_collapsed.clear();
            self.selected_commit_file = None;
            self.commit_file_diff = None;
            self.loading_commit_files = false;
        }
        // 检查 tab 是否已存在
        let existing = self
            .file_tabs
            .iter()
            .position(|t| t.path == path && t.source == FileTabSource::Changes(kind));
        if let Some(idx) = existing {
            self.active_file_tab_idx = Some(idx);
            self.selected_file = Some((path.clone(), kind));
            self.selected_pf_path = None;
            self.current_file_content = None;
            if let Some(cached) = self.file_tabs[idx].cached_diff.clone() {
                // 命中缓存，直接展示
                self.current_diff = Some(cached);
                self.loading_diff = false;
                cx.notify();
                return;
            }
            // Tab 存在但无缓存（如切换 ignore-whitespace 后清掉了）→ 继续拉取
            self.current_diff = None;
            self.loading_diff = true;
        } else {
            // 新 tab
            self.file_tabs.push(FileTab {
                path: path.clone(),
                source: FileTabSource::Changes(kind),
                cached_diff: None,
                cached_content: None,
            });
            self.active_file_tab_idx = Some(self.file_tabs.len() - 1);
            self.selected_file = Some((path.clone(), kind));
            self.selected_pf_path = None;
            self.current_file_content = None;
            self.current_diff = None;
            self.loading_diff = true;
        }
        cx.notify();

        // Untracked / Conflict 暂不渲染 diff
        let diff_kind = match kind {
            GroupKind::Staged => DiffKind::IndexVsHead,
            GroupKind::Unstaged => DiffKind::WorkingTreeVsIndex,
            GroupKind::Untracked | GroupKind::Conflict => {
                self.loading_diff = false;
                cx.notify();
                return;
            }
        };
        let driver = self.driver.clone();
        let path_for_diff = path.clone();
        let ignore_ws = self.diff_ignore_whitespace;
        let context_lines = self.diff_view_mode.context_lines();
        let active_idx = self.active_file_tab_idx;
        cx.spawn(async move |this, cx| {
            let result = driver
                .diff_file_full_opts(&repo, &path_for_diff, diff_kind, ignore_ws, context_lines)
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading_diff = false;
                match result {
                    Ok(d) => {
                        this.current_diff = Some(d.clone());
                        // 缓存到对应 tab
                        if let Some(idx) = active_idx
                            && let Some(tab) = this.file_tabs.get_mut(idx)
                            && tab.path == path_for_diff
                        {
                            tab.cached_diff = Some(d);
                        }
                    }
                    Err(e) => {
                        error!(error = %e, path = %path_for_diff, "vcs: diff failed");
                        this.error = Some(format!("拉取 diff 失败：{e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 关闭指定索引的文件 tab；根据剩余 active tab 的 source 重置 diff/pf 字段
    pub(super) fn close_file_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.file_tabs.len() {
            return;
        }
        self.file_tabs.remove(idx);
        if self.file_tabs.is_empty() {
            self.active_file_tab_idx = None;
            self.selected_file = None;
            self.current_diff = None;
            self.loading_diff = false;
            self.selected_pf_path = None;
            self.current_file_content = None;
            self.loading_file_content = false;
            self.selected_commit_file = None;
        } else {
            let new_idx = match self.active_file_tab_idx {
                Some(i) if i == idx => idx.saturating_sub(1).min(self.file_tabs.len() - 1),
                Some(i) if i > idx => i - 1,
                Some(i) => i,
                None => 0,
            };
            self.active_file_tab_idx = Some(new_idx);
            if let Some(tab) = self.file_tabs.get(new_idx) {
                self.activate_file_tab_state(tab.clone());
            }
        }
        cx.notify();
    }

    /// 同步 active tab 的派生状态：根据 source 写 selected_file / selected_pf_path 等
    pub(super) fn activate_file_tab_state(&mut self, tab: FileTab) {
        match &tab.source {
            FileTabSource::Changes(kind) => {
                self.selected_file = Some((tab.path.clone(), *kind));
                self.current_diff = tab.cached_diff.clone();
                self.loading_diff = tab.cached_diff.is_none()
                    && matches!(kind, GroupKind::Staged | GroupKind::Unstaged);
                self.selected_pf_path = None;
                self.current_file_content = None;
                self.loading_file_content = false;
                self.selected_commit_file = None;
            }
            FileTabSource::ProjectFiles => {
                self.selected_pf_path = Some(tab.path.clone());
                self.current_file_content = tab.cached_content.clone();
                self.loading_file_content = tab.cached_content.is_none();
                self.selected_file = None;
                self.current_diff = None;
                self.loading_diff = false;
                self.selected_commit_file = None;
            }
            FileTabSource::Commit { .. } => {
                // commit tab：复用 current_diff 渲染（与 Changes 同一路径）
                self.selected_file = None;
                self.current_diff = tab.cached_diff.clone();
                self.loading_diff = tab.cached_diff.is_none();
                self.selected_pf_path = None;
                self.current_file_content = None;
                self.loading_file_content = false;
                self.selected_commit_file = Some(tab.path.clone());
            }
        }
    }
}
