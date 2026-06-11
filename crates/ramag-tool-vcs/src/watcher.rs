//! 仓库文件系统监听：外部改动（编辑器保存 / 终端 git 操作）→ 过滤 + 防抖 → 通知刷新。
//! 配合 refresh_workspace_silent 的 status 指纹比对，无实质变化时 UI 零扰动

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

/// 事件静默 800ms 后才触发回调（尾沿防抖）：编辑器批量保存 / git 写库的连发事件合并为一次
const DEBOUNCE: Duration = Duration::from_millis(800);

/// 监听句柄：drop 即停止监听，防抖线程随通道关闭自动退出
pub(crate) struct RepoWatcher {
    _watcher: RecommendedWatcher,
}

impl RepoWatcher {
    /// 递归监听 repo_root；`on_change` 在防抖线程上调用（调用方负责切回 UI 线程）
    pub(crate) fn start(
        repo_root: PathBuf,
        on_change: impl Fn() + Send + 'static,
    ) -> notify::Result<Self> {
        let (tx, rx) = mpsc::channel::<()>();
        let root_for_filter = repo_root.clone();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let Ok(event) = res else { return };
                if event.paths.iter().any(|p| is_relevant(&root_for_filter, p)) {
                    let _ = tx.send(());
                }
            })?;
        watcher.watch(&repo_root, RecursiveMode::Recursive)?;

        std::thread::spawn(move || {
            // 首个事件到达后，持续吸收事件直到静默满 DEBOUNCE，合并为一次回调
            while rx.recv().is_ok() {
                loop {
                    match rx.recv_timeout(DEBOUNCE) {
                        Ok(()) => continue,
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                        // watcher 已 drop：线程退出
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
                on_change();
            }
        });
        Ok(Self { _watcher: watcher })
    }
}

/// 事件过滤：工作区文件一律相关；`.git` 内部只放行表示仓库状态变化的关键路径，
/// 屏蔽 objects / logs / COMMIT_EDITMSG / *.lock 等高频噪声
fn is_relevant(root: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return true;
    };
    let mut comps = rel.components().map(|c| c.as_os_str().to_string_lossy());
    let Some(first) = comps.next() else {
        return false;
    };
    if first != ".git" {
        return true;
    }
    let Some(second) = comps.next() else {
        return false;
    };
    matches!(
        second.as_ref(),
        // index 变化 = stage/unstage/commit；HEAD/refs = 分支移动；其余 = 进行中操作标记
        "index"
            | "HEAD"
            | "ORIG_HEAD"
            | "MERGE_HEAD"
            | "CHERRY_PICK_HEAD"
            | "REVERT_HEAD"
            | "refs"
            | "rebase-merge"
            | "rebase-apply"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(p: &str) -> bool {
        is_relevant(Path::new("/repo"), Path::new(p))
    }

    #[test]
    fn worktree_files_are_relevant() {
        assert!(rel("/repo/src/main.rs"));
        assert!(rel("/repo/README.md"));
    }

    #[test]
    fn git_state_files_are_relevant() {
        assert!(rel("/repo/.git/index"));
        assert!(rel("/repo/.git/HEAD"));
        assert!(rel("/repo/.git/refs/heads/main"));
        assert!(rel("/repo/.git/MERGE_HEAD"));
    }

    #[test]
    fn git_noise_is_filtered() {
        assert!(!rel("/repo/.git/objects/ab/cdef123"));
        assert!(!rel("/repo/.git/logs/HEAD"));
        assert!(!rel("/repo/.git/COMMIT_EDITMSG"));
        assert!(!rel("/repo/.git/index.lock"));
        assert!(!rel("/repo/.git/config"));
    }
}
