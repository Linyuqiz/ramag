//! 工作区状态同步：静默刷新（窗口激活 / 手动刷新）+ Changes 文件 tabs 与最新 status 对齐

use gpui::Context;
use ramag_domain::entities::{BranchKind, FileChangeKind, FileStatus};

use super::helpers::{FileTabSource, GroupKind};
use super::vcs_view::VcsView;

impl VcsView {
    /// 静默刷新工作区：status + 本地/远程分支 + 当前 Files 视图数据。
    /// 不显示整屏 loading；完成后对齐 Changes tabs（外部改动 / 终端 git 操作后界面自动跟上）
    pub(super) fn refresh_workspace_silent(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        cx.spawn(async move |this, cx| {
            let status_fut = driver.status(&repo);
            let local_fut = driver.list_branches(&repo, BranchKind::Local);
            let remote_fut = driver.list_branches(&repo, BranchKind::Remote);
            let (status, local, remote) =
                futures::future::join3(status_fut, local_fut, remote_fut).await;
            let _ = this.update(cx, |this, cx| {
                if !this.is_current_repo(&repo) {
                    return;
                }
                // 文件状态指纹没变 → 跳过 tabs 对齐和 diff 重拉，避免窗口激活白闪一次
                let files_changed = match (&this.status, &status) {
                    (Some(old), Ok(new)) => {
                        files_fingerprint(&old.files) != files_fingerprint(&new.files)
                    }
                    _ => true,
                };
                if let Ok(s) = status {
                    this.status = Some(s);
                }
                if let Ok(b) = local {
                    this.local_branches = b;
                }
                if let Ok(b) = remote {
                    this.remote_branches = b;
                }
                if files_changed {
                    // Project Files 内容缓存随外部改动失效（重激活 tab 时按需重读）
                    for tab in &mut this.file_tabs {
                        if matches!(tab.source, FileTabSource::ProjectFiles) {
                            tab.cached_content = None;
                        }
                    }
                    this.sync_changes_tabs_with_status(cx);
                    // active 是 PF tab 时立即重读（sync 只处理 Changes 来源的重拉）
                    if let Some(idx) = this.active_file_tab_idx
                        && let Some(tab) = this.file_tabs.get(idx).cloned()
                        && matches!(tab.source, FileTabSource::ProjectFiles)
                    {
                        this.select_pf_file(tab.path, cx);
                    }
                    // Project / Stash 模式的列表数据独立于 status，单独拉
                    match this.files_view_mode {
                        super::helpers::FilesViewMode::Project => this.reload_project_files(cx),
                        super::helpers::FilesViewMode::Stash => this.reload_stashes(cx),
                        super::helpers::FilesViewMode::Changes => {}
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 启动当前仓库的文件系统监听：外部改动防抖后静默刷新。
    /// 旧 watcher 先 drop（防抖线程随通道关闭退出，旧转发任务随 sender 关闭结束）
    pub(in crate::views) fn start_fs_watcher(&mut self, cx: &mut Context<Self>) {
        self.fs_watcher = None;
        let Some(repo) = self.repo.as_ref() else {
            return;
        };
        let root = std::path::PathBuf::from(&repo.path);
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<()>();
        match crate::watcher::RepoWatcher::start(root, move || {
            let _ = tx.unbounded_send(());
        }) {
            Ok(w) => {
                self.fs_watcher = Some(w);
                cx.spawn(async move |this, cx| {
                    use futures::StreamExt as _;
                    while rx.next().await.is_some() {
                        let alive = this.update(cx, |this, cx| {
                            // busy 中跳过：写操作完成路径自己会刷新，避免叠加
                            if this.repo.is_some() && !this.loading && !this.busy {
                                this.refresh_workspace_silent(cx);
                            }
                        });
                        if alive.is_err() {
                            break;
                        }
                    }
                })
                .detach();
            }
            Err(e) => {
                // 监听失败不阻断使用：窗口激活刷新 + 手动刷新仍可用
                tracing::warn!(error = %e, "vcs: fs watcher start failed");
            }
        }
    }

    /// 把 Changes 来源的文件 tabs 与最新 `self.status` 对齐：
    /// - 文件已无任何变更 → 关闭 tab（diff 必为空，保留无意义）
    /// - 文件变更组别迁移（如 stage 后 Unstaged → Staged）→ 重定向 tab 的 GroupKind
    /// - 保留的 tab 一律清 diff 缓存（状态变过，旧 diff 不可信）；active 是 Changes 时重拉
    ///
    /// ProjectFiles / Commit 来源的 tabs 不受影响（仅索引可能因关闭前移）
    pub(super) fn sync_changes_tabs_with_status(&mut self, cx: &mut Context<Self>) {
        let Some(status) = self.status.as_ref() else {
            return;
        };
        let active_identity = self
            .active_file_tab_idx
            .and_then(|i| self.file_tabs.get(i))
            .map(|t| (t.path.clone(), t.source.clone()));

        let files = status.files.clone();
        let mut new_tabs = Vec::with_capacity(self.file_tabs.len());
        for mut tab in std::mem::take(&mut self.file_tabs) {
            let FileTabSource::Changes(kind) = tab.source else {
                new_tabs.push(tab);
                continue;
            };
            let Some(f) = files.iter().find(|f| f.path == tab.path) else {
                continue;
            };
            let new_kind = redirect_group_kind(f, kind);
            // 重定向后可能与既有 tab 重合（如 Staged + Unstaged 两个 tab 合流）→ 去重
            if new_tabs.iter().any(|t: &super::helpers::FileTab| {
                t.path == tab.path && t.source == FileTabSource::Changes(new_kind)
            }) {
                continue;
            }
            tab.source = FileTabSource::Changes(new_kind);
            tab.cached_diff = None;
            new_tabs.push(tab);
        }
        self.file_tabs = new_tabs;

        // 恢复 active tab：优先同 (path, source)，其次同 path 的 Changes tab，再次序号回退
        let restored = active_identity.and_then(|(path, source)| {
            self.file_tabs
                .iter()
                .position(|t| t.path == path && t.source == source)
                .or_else(|| {
                    matches!(source, FileTabSource::Changes(_))
                        .then(|| {
                            self.file_tabs.iter().position(|t| {
                                t.path == path && matches!(t.source, FileTabSource::Changes(_))
                            })
                        })
                        .flatten()
                })
        });
        match restored {
            Some(idx) => {
                self.active_file_tab_idx = Some(idx);
                let tab = self.file_tabs[idx].clone();
                match tab.source {
                    // Changes：缓存已清，走 select_file 重拉（占位/伪 diff 由其内部分支处理）
                    FileTabSource::Changes(kind) => self.select_file(tab.path, kind, cx),
                    // 其余来源缓存未动，仅同步派生字段
                    _ => self.activate_file_tab_state(tab),
                }
            }
            None => {
                // active tab 被关：顺延到最后一个 tab；没有 tab 则清空主区
                self.active_file_tab_idx = self.file_tabs.len().checked_sub(1);
                if let Some(idx) = self.active_file_tab_idx {
                    let tab = self.file_tabs[idx].clone();
                    match tab.source {
                        FileTabSource::Changes(kind) => self.select_file(tab.path, kind, cx),
                        _ => self.activate_file_tab_state(tab),
                    }
                } else {
                    self.selected_file = None;
                    self.current_diff = None;
                    self.loading_diff = false;
                    self.selected_pf_path = None;
                    self.current_file_content = None;
                    self.loading_file_content = false;
                    self.selected_commit_file = None;
                }
            }
        }
        cx.notify();
    }
}

/// 文件状态指纹：路径 + 暂存/工作区变更类型。两次 status 指纹一致 = 工作区无实质变化
fn files_fingerprint(
    files: &[FileStatus],
) -> Vec<(&str, Option<FileChangeKind>, Option<FileChangeKind>)> {
    files
        .iter()
        .map(|f| (f.path.as_str(), f.staged, f.unstaged))
        .collect()
}

/// 按最新文件状态推导 tab 应归属的组：原组仍有效则保持，否则按 冲突 > 已暂存 > 未暂存 > 未跟踪 迁移
fn redirect_group_kind(f: &FileStatus, prefer: GroupKind) -> GroupKind {
    if f.is_conflicted() {
        return GroupKind::Conflict;
    }
    let staged_ok = f.staged.is_some();
    let untracked = matches!(f.unstaged, Some(FileChangeKind::Untracked));
    let unstaged_ok = f.unstaged.is_some() && !untracked;
    let valid = |k: GroupKind| match k {
        GroupKind::Staged => staged_ok,
        GroupKind::Unstaged => unstaged_ok,
        GroupKind::Untracked => untracked,
        GroupKind::Conflict => false,
    };
    if valid(prefer) {
        prefer
    } else if staged_ok {
        GroupKind::Staged
    } else if unstaged_ok {
        GroupKind::Unstaged
    } else {
        GroupKind::Untracked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs(staged: Option<FileChangeKind>, unstaged: Option<FileChangeKind>) -> FileStatus {
        FileStatus {
            path: "a.rs".into(),
            old_path: None,
            staged,
            unstaged,
        }
    }

    #[test]
    fn keeps_valid_group() {
        // 同文件先 add 再改：两组都有效，各自保持原组
        let f = fs(
            Some(FileChangeKind::Modified),
            Some(FileChangeKind::Modified),
        );
        assert_eq!(
            redirect_group_kind(&f, GroupKind::Staged),
            GroupKind::Staged
        );
        assert_eq!(
            redirect_group_kind(&f, GroupKind::Unstaged),
            GroupKind::Unstaged
        );
    }

    #[test]
    fn stage_moves_unstaged_tab_to_staged() {
        let f = fs(Some(FileChangeKind::Modified), None);
        assert_eq!(
            redirect_group_kind(&f, GroupKind::Unstaged),
            GroupKind::Staged
        );
    }

    #[test]
    fn unstage_moves_staged_tab_back() {
        let f = fs(None, Some(FileChangeKind::Modified));
        assert_eq!(
            redirect_group_kind(&f, GroupKind::Staged),
            GroupKind::Unstaged
        );
    }

    #[test]
    fn staging_untracked_redirects_to_staged() {
        let f = fs(Some(FileChangeKind::Added), None);
        assert_eq!(
            redirect_group_kind(&f, GroupKind::Untracked),
            GroupKind::Staged
        );
    }

    #[test]
    fn conflict_wins_over_everything() {
        let f = fs(Some(FileChangeKind::Conflicted), None);
        assert_eq!(
            redirect_group_kind(&f, GroupKind::Unstaged),
            GroupKind::Conflict
        );
    }
}
