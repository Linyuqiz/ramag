//! VcsView::new：所有 InputState / Resizable / Scroll handle 字段初始化

use std::cell::RefCell;
use std::sync::Arc;

use gpui::{AppContext as _, Context, ScrollHandle, UniformListScrollHandle, Window};
use gpui_component::{
    input::{InputEvent, InputState},
    resizable::ResizableState,
};
use ramag_domain::traits::{GitDriver, Storage};

use super::super::helpers::{ActiveView, DiffViewMode, FilesViewMode, ViewMode};
use super::VcsView;

impl VcsView {
    pub fn new(
        driver: Arc<dyn GitDriver>,
        storage: Arc<dyn Storage>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let commit_input = cx.new(|cx_inner| {
            InputState::new(window, cx_inner)
                .multi_line(true)
                .rows(3)
                .placeholder("commit message（首行 subject，空行后写 body）")
        });
        let create_branch_input = cx.new(|cx_inner| {
            InputState::new(window, cx_inner).placeholder("新分支名（基于当前 HEAD）")
        });
        let create_tag_input = cx.new(|cx_inner| {
            InputState::new(window, cx_inner).placeholder("新 tag 名（基于当前 HEAD）")
        });
        let create_tag_message_input = cx.new(|cx_inner| {
            InputState::new(window, cx_inner).placeholder("message（可选，填了就是 annotated tag）")
        });
        let history_search_input = cx.new(|cx_inner| {
            InputState::new(window, cx_inner).placeholder("搜索：关键词 / @作者 / 7d/1m 时间下限")
        });
        let clone_url_input = cx.new(|cx_inner| {
            InputState::new(window, cx_inner).placeholder("仓库 URL（HTTPS / SSH）")
        });
        let ide_left_resize = cx.new(|_| ResizableState::default());
        let ide_files_resize = cx.new(|_| ResizableState::default());
        let detail_resize = cx.new(|_| ResizableState::default());
        let repo_search_input = cx.new(|cx_inner| {
            InputState::new(window, cx_inner).placeholder("搜索仓库（名称 / 路径）")
        });
        let files_search_input =
            cx.new(|cx_inner| InputState::new(window, cx_inner).placeholder("搜索文件路径"));
        // 订阅搜索框 Change → notify 主 view 重渲染（触发文件过滤即时反馈）
        cx.subscribe(
            &files_search_input,
            |_this: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            },
        )
        .detach();
        let this = Self {
            driver,
            storage,
            repo: None,
            status: None,
            local_branches: Vec::new(),
            remote_branches: Vec::new(),
            error: None,
            loading: false,
            busy: false,
            commit_input,
            commit_amend: false,
            commit_sign: false,
            pending_commit_text: None,
            selected_file: None,
            current_diff: None,
            loading_diff: false,
            view_mode: ViewMode::Workspace,
            history_commits: Vec::new(),
            history_has_more: false,
            loading_history: false,
            stashes: Vec::new(),
            loading_stashes: false,
            create_branch_input,
            create_branch_base: None,
            tags: Vec::new(),
            loading_tags: false,
            create_tag_input,
            create_tag_message_input,
            collapsed_local: false,
            collapsed_remote: true,
            collapsed_tag: true,
            selected_diff_lines: std::collections::HashSet::new(),
            expanded_diff_spacers: std::collections::HashSet::new(),
            remotes: Vec::new(),
            loading_remotes: false,
            viewing_commit: None,
            commit_files: Vec::new(),
            selected_commit_file: None,
            commit_file_diff: None,
            loading_commit_files: false,
            commit_files_collapsed: std::collections::HashSet::new(),
            changes_collapsed_dirs: std::collections::HashSet::new(),
            history_path_filter: None,
            history_search_input,
            blame_lines: Vec::new(),
            loading_blame: false,
            showing_blame: false,
            inline_blame_text: None,
            diff_ignore_whitespace: false,
            diff_view_mode: DiffViewMode::Standard,
            reflog_entries: Vec::new(),
            loading_reflog: false,
            showing_reflog: false,
            ide_left_resize,
            ide_files_resize,
            detail_resize,
            active_view: ActiveView::RepoList,
            recent_repos: Vec::new(),
            repo_search_input,
            files_view_mode: FilesViewMode::Project,
            files_search_input,
            project_files: Vec::new(),
            loading_project_files: false,
            project_expanded_dirs: std::collections::HashSet::new(),
            project_files_version: 0,
            project_expanded_dirs_version: 0,
            project_rows_cache: RefCell::new(None),
            project_scroll: UniformListScrollHandle::new(),
            selected_pf_path: None,
            current_file_content: None,
            loading_file_content: false,
            pf_content_scroll: UniformListScrollHandle::new(),
            diff_scroll: UniformListScrollHandle::new(),
            commit_files_scroll: UniformListScrollHandle::new(),
            conflict_ours_scroll: UniformListScrollHandle::new(),
            conflict_theirs_scroll: UniformListScrollHandle::new(),
            history_scroll: UniformListScrollHandle::new(),
            reflog_scroll: UniformListScrollHandle::new(),
            pf_content_h_scroll: ScrollHandle::new(),
            diff_h_scroll: ScrollHandle::new(),
            diff_h_scroll_right: ScrollHandle::new(),
            history_pane_visible: false,
            open_repos: Vec::new(),
            file_tabs: Vec::new(),
            active_file_tab_idx: None,
            repo_session_cache: std::collections::HashMap::new(),
            clone_url_input,
            clone_dest_path: None,
            show_clone_panel: false,
            show_rebase_plan: false,
            rebase_plan_onto: String::new(),
            rebase_todos: Vec::new(),
            loading_rebase_plan: false,
            conflict_editor_path: None,
            conflict_content: None,
            loading_conflict: false,
            focus_handle: cx.focus_handle(),
        };
        Self::load_recent_repos_async(cx);
        this
    }
}
