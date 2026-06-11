//! 文件 tab：select_file / close_file_tab / activate_file_tab_state / untracked 预览

use gpui::Context;
use ramag_domain::entities::{DiffKind, DiffLine, DiffLineKind, FileChangeKind, FileDiff, Hunk};
use tracing::error;

use super::helpers::{FileTab, FileTabSource, GroupKind};
use super::vcs_view::VcsView;
use super::vcs_view_ops_repo::{RawFileContent, read_raw_file_content};

impl VcsView {
    /// 选中文件查看 diff（Changes 模式）：tab 已存在则复用并优先展示缓存；否则新开 tab + 异步拉
    pub(super) fn select_file(&mut self, path: String, kind: GroupKind, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        // 切换文件 → 清 spacer 展开态（hunk_idx 随 diff 变化，跨文件保留无意义）
        self.expanded_diff_spacers.clear();
        // 横滚归位，否则新文件停在上个文件的横滚位置、看不到行首
        self.diff_h_scroll
            .set_offset(gpui::point(gpui::px(0.0), gpui::px(0.0)));
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

        let diff_kind = match kind {
            GroupKind::Staged => DiffKind::IndexVsHead,
            GroupKind::Unstaged => DiffKind::WorkingTreeVsIndex,
            // Untracked 不在 index：git diff 无输出 → 读盘构造「全新增」伪 diff 预览
            GroupKind::Untracked => {
                self.load_untracked_preview(path, cx);
                return;
            }
            // Conflict 走三栏解决器（左侧行点击直达），diff 区仅给提示
            GroupKind::Conflict => {
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
                if !this.is_current_repo(&repo) {
                    cx.notify();
                    return;
                }
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

    /// 读盘构造 untracked 文件的「全新增」伪 diff：与普通 diff 同一渲染管线，
    /// 新文件点开即可预览内容（之前是占位文案，必须先 Stage 才能看）
    fn load_untracked_preview(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref() else {
            return;
        };
        let repo_id = repo.id.clone();
        let abs_path = std::path::PathBuf::from(&repo.path).join(&path);
        cx.spawn(async move |this, cx| {
            let (tx, rx) = futures::channel::oneshot::channel();
            let rel_for_thread = path.clone();
            std::thread::spawn(move || {
                let raw = read_raw_file_content(&abs_path, &rel_for_thread);
                let _ = tx.send(raw);
            });
            let raw = rx.await.ok();
            let _ = this.update(cx, |this, cx| {
                this.loading_diff = false;
                if !this.is_current_repo(&repo_id) {
                    cx.notify();
                    return;
                }
                match raw {
                    Some(raw) if raw.error.is_none() => {
                        let d = build_untracked_diff(raw);
                        if let Some(tab) = this.file_tabs.iter_mut().find(|t| {
                            t.path == path
                                && t.source == FileTabSource::Changes(GroupKind::Untracked)
                        }) {
                            tab.cached_diff = Some(d.clone());
                        }
                        let is_selected = this
                            .selected_file
                            .as_ref()
                            .is_some_and(|(p, k)| p == &path && *k == GroupKind::Untracked);
                        if is_selected {
                            this.current_diff = Some(d);
                        }
                    }
                    Some(raw) => {
                        let msg = raw.error.unwrap_or_else(|| "未知错误".into());
                        error!(error = %msg, path = %path, "vcs: read untracked file failed");
                        this.error = Some(format!("读取文件失败：{msg}"));
                    }
                    None => {
                        this.error = Some("读取文件失败：内部通道中断".into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
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

/// 文件内容 → 「全新增」伪 diff：单 hunk，每行 Add；二进制走 FileDiff.binary 占位；
/// 截断（>4MB）通过 hunk heading 提示
fn build_untracked_diff(raw: RawFileContent) -> FileDiff {
    let lines: Vec<DiffLine> = raw
        .lines
        .into_iter()
        .enumerate()
        .map(|(i, text)| DiffLine {
            kind: DiffLineKind::Add,
            old_lineno: None,
            new_lineno: Some(i as u32 + 1),
            text,
        })
        .collect();
    let hunks = if lines.is_empty() {
        Vec::new()
    } else {
        vec![Hunk {
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: lines.len() as u32,
            heading: raw
                .truncated
                .then(|| "文件过大，预览已截断（前 4MB）".to_string()),
            lines,
        }]
    };
    FileDiff {
        path: raw.path,
        old_path: None,
        change_kind: FileChangeKind::Untracked,
        binary: raw.binary,
        old_mode: None,
        new_mode: None,
        hunks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(lines: Vec<&str>, binary: bool, truncated: bool) -> RawFileContent {
        RawFileContent {
            path: "new.rs".into(),
            lines: lines.into_iter().map(str::to_owned).collect(),
            truncated,
            binary,
            error: None,
        }
    }

    #[test]
    fn untracked_diff_marks_all_lines_added() {
        let d = build_untracked_diff(raw(vec!["a", "b"], false, false));
        assert_eq!(d.hunks.len(), 1);
        let hunk = &d.hunks[0];
        assert_eq!(hunk.new_lines, 2);
        assert!(
            hunk.lines
                .iter()
                .all(|l| matches!(l.kind, DiffLineKind::Add))
        );
        assert_eq!(hunk.lines[1].new_lineno, Some(2));
        assert_eq!(hunk.lines[1].old_lineno, None);
    }

    #[test]
    fn untracked_diff_binary_has_no_hunks() {
        let d = build_untracked_diff(raw(vec![], true, false));
        assert!(d.binary);
        assert!(d.hunks.is_empty());
    }

    #[test]
    fn untracked_diff_truncated_sets_heading() {
        let d = build_untracked_diff(raw(vec!["x"], false, true));
        assert!(d.hunks[0].heading.as_deref().unwrap_or("").contains("截断"));
    }
}
