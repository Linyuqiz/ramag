//! GPUI 渲染测试：headless 在内存渲染 VcsView（含 diff session 态），
//! 验证整条 diff 渲染管线（FileDiff → build_split_keys → element → 布局 → paint）不 panic。
//! 截图被 macOS 屏幕录制权限挡，本测试是 UI 渲染层的可重复真机验证替代。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use gpui::TestAppContext;
use ramag_domain::entities::{
    Branch, BranchKind, Commit, ConnectionConfig, ConnectionId, DiffKind, DiffLine, DiffLineKind,
    FileChangeKind, FileDiff, FileStatus, Hunk, LogOptions, QueryRecord, QueryRecordId, RepoConfig,
    RepoId, WorkingTreeStatus,
};
use ramag_domain::error::{DomainError, Result};
use ramag_domain::traits::{GitDriver, Storage};

use super::super::helpers::{ActiveView, FileTab, FileTabSource, GroupKind};
use super::VcsView;

/// 空壳 GitDriver：render 是纯展示、不调 driver，方法只需可编译且不 panic
struct MockGit;

#[async_trait]
impl GitDriver for MockGit {
    fn name(&self) -> &'static str {
        "mock"
    }
    async fn open_repo(&self, _: &Path) -> Result<RepoConfig> {
        Err(DomainError::NotImplemented("mock".into()))
    }
    async fn close_repo(&self, _: &RepoId) -> Result<()> {
        Ok(())
    }
    async fn status(&self, _: &RepoId) -> Result<WorkingTreeStatus> {
        Err(DomainError::NotImplemented("mock".into()))
    }
    async fn list_branches(&self, _: &RepoId, _: BranchKind) -> Result<Vec<Branch>> {
        Ok(vec![])
    }
    async fn log(&self, _: &RepoId, _: LogOptions) -> Result<Vec<Commit>> {
        Ok(vec![])
    }
    async fn diff_file(&self, _: &RepoId, _: &str, _: DiffKind) -> Result<FileDiff> {
        Err(DomainError::NotImplemented("mock".into()))
    }
}

/// 空壳 Storage：render 不调 storage
struct MockStorage;

#[async_trait]
impl Storage for MockStorage {
    async fn list_connections(&self) -> Result<Vec<ConnectionConfig>> {
        Ok(vec![])
    }
    async fn get_connection(&self, _: &ConnectionId) -> Result<Option<ConnectionConfig>> {
        Ok(None)
    }
    async fn save_connection(&self, _: &ConnectionConfig) -> Result<()> {
        Ok(())
    }
    async fn delete_connection(&self, _: &ConnectionId) -> Result<()> {
        Ok(())
    }
    async fn append_history(&self, _: &QueryRecord) -> Result<()> {
        Ok(())
    }
    async fn list_history(&self, _: Option<&ConnectionId>, _: usize) -> Result<Vec<QueryRecord>> {
        Ok(vec![])
    }
    async fn delete_history(&self, _: &QueryRecordId) -> Result<()> {
        Ok(())
    }
    async fn clear_history(&self, _: Option<&ConnectionId>) -> Result<()> {
        Ok(())
    }
    async fn get_preference(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
    async fn set_preference(&self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
}

fn mock_repo() -> RepoConfig {
    RepoConfig {
        id: RepoId::new(),
        name: "test-repo".into(),
        path: "/tmp/test-repo".into(),
        last_opened_at: None,
        favorite: false,
    }
}

fn dline(kind: DiffLineKind, old: Option<u32>, new: Option<u32>, text: &str) -> DiffLine {
    DiffLine {
        kind,
        old_lineno: old,
        new_lineno: new,
        text: text.into(),
    }
}

/// 含 context + delete + add 的多行 diff（触发 split 双栏配对渲染）。
/// 用 `.rs` 路径 + 真 Rust 代码行，让语法高亮路径（SyntaxHighlighter）参与渲染验证。
fn test_diff() -> FileDiff {
    FileDiff {
        path: "a.rs".into(),
        old_path: None,
        change_kind: FileChangeKind::Modified,
        binary: false,
        old_mode: None,
        new_mode: None,
        hunks: vec![Hunk {
            old_start: 1,
            old_lines: 3,
            new_start: 1,
            new_lines: 3,
            heading: None,
            lines: vec![
                dline(DiffLineKind::Context, Some(1), Some(1), "fn main() {"),
                dline(DiffLineKind::Delete, Some(2), None, "    let x = 1;"),
                dline(DiffLineKind::Add, None, Some(2), "    let y = 2;"),
                dline(DiffLineKind::Context, Some(3), Some(3), "}"),
            ],
        }],
    }
}

fn mock_status() -> WorkingTreeStatus {
    WorkingTreeStatus {
        head_branch: Some("main".into()),
        head_commit: Some("abc1234".into()),
        files: vec![FileStatus {
            path: "a.rs".into(),
            old_path: None,
            staged: None,
            unstaged: Some(FileChangeKind::Modified),
        }],
        ..Default::default()
    }
}

/// 注入「打开仓库 + 选中改动文件 + diff 已加载」的 Session 态
fn inject_diff_session(v: &mut VcsView) {
    let repo = mock_repo();
    v.open_repos = vec![repo.clone()];
    v.repo = Some(repo);
    v.active_view = ActiveView::Session;
    v.status = Some(mock_status());
    v.current_diff = Some(test_diff());
    v.selected_file = Some(("a.rs".into(), GroupKind::Unstaged));
    v.file_tabs = vec![FileTab {
        path: "a.rs".into(),
        source: FileTabSource::Changes(GroupKind::Unstaged),
        cached_diff: Some(test_diff()),
        cached_content: None,
    }];
    v.active_file_tab_idx = Some(0);
}

/// 渲染整条 IDE 布局（含 diff split 5-list：左 gutter/content + 中间列 + 右 gutter/content + 行配对 + scroll）不 panic。
/// 能跑完 add_window_view（内部 draw）+ run_until_parked 即证明渲染管线健康。
#[gpui::test]
fn vcs_view_renders_diff_split_without_panic(cx: &mut TestAppContext) {
    // 渲染 gpui-component 视图前必须 init（cx.theme() 取 Theme 全局，缺则 panic）
    cx.update(gpui_component::init);

    let (view, cx) = cx.add_window_view(|window, cx| {
        VcsView::new(Arc::new(MockGit), Arc::new(MockStorage), window, cx)
    });

    view.update(cx, |v, cx| {
        inject_diff_session(v);
        cx.notify();
    });
    cx.run_until_parked();

    view.read_with(cx, |v, _| {
        assert!(v.current_diff.is_some(), "diff 应已注入并参与渲染");
        assert_eq!(v.file_tabs.len(), 1, "应有 1 个文件 tab");
    });

    // 再渲染一帧（状态不变），验证幂等不崩
    view.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
}

/// 切到「全文件」diff 视图模式后仍能渲染（context_lines 路径）
#[gpui::test]
fn vcs_view_renders_full_file_diff_mode(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (view, cx) = cx.add_window_view(|window, cx| {
        VcsView::new(Arc::new(MockGit), Arc::new(MockStorage), window, cx)
    });
    view.update(cx, |v, cx| {
        inject_diff_session(v);
        v.diff_view_mode = super::super::helpers::DiffViewMode::FullFile;
        cx.notify();
    });
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(matches!(
            v.diff_view_mode,
            super::super::helpers::DiffViewMode::FullFile
        ));
    });
}
