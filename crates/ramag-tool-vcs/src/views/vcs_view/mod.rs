//! VcsView：Git 主视图。状态 + Render + active_view 路由

mod new;
mod render;

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, ScrollHandle, SharedString,
    UniformListScrollHandle,
};
use gpui_component::{input::InputState, resizable::ResizableState};
use ramag_domain::entities::{
    Branch, Commit, ConflictContent, FileDiff, FileStatus, RebaseTodo, Remote, RepoConfig, Stash,
    Tag, WorkingTreeStatus,
};
use ramag_domain::traits::{GitDriver, Storage};

use super::helpers::{
    ActiveView, DiffViewMode, FileContentSnapshot, FileTab, FilesViewMode, GroupKind, ViewMode,
};
use super::project_files::ProjectRowsCacheEntry;

/// 仓库 tab UI 状态快照：文件 tabs + commit 草稿（按仓库隔离，避免串扰）
/// commit 文本通过 `pending_commit_text` + Render 内 `cx.defer_in` 写回 InputState
#[derive(Clone, Default)]
pub(super) struct RepoSessionState {
    pub file_tabs: Vec<FileTab>,
    pub active_file_tab_idx: Option<usize>,
    pub commit_text: SharedString,
    pub commit_amend: bool,
    pub commit_sign: bool,
}

#[derive(Debug, Clone)]
pub enum VcsEvent {
    /// 预留：未来从 home 跳转打开特定仓库时用
    OpenRepo(PathBuf),
}

/// 主视图状态。字段标 `pub(super)` 让兄弟子模块跨 mod 访问。
pub struct VcsView {
    pub(super) driver: Arc<dyn GitDriver>,
    /// 持久化层（recent_repos 跨重启保留）；按 RepoId 单条 CRUD（redb `repos` 表）
    pub(super) storage: Arc<dyn Storage>,
    /// 当前已打开的仓库（None = 还没选）
    pub(super) repo: Option<RepoConfig>,
    /// 工作区状态快照
    pub(super) status: Option<WorkingTreeStatus>,
    /// 本地分支列表
    pub(super) local_branches: Vec<Branch>,
    /// 远程分支列表
    pub(super) remote_branches: Vec<Branch>,
    /// 错误信息（打开 / 查询失败时显示）
    pub(super) error: Option<String>,
    /// 是否正在加载（点选目录后 → 各 RPC 完成前）
    pub(super) loading: bool,
    /// 写操作正在进行中（stage / unstage / discard / commit）：避免重复点击
    pub(super) busy: bool,
    /// commit message 输入框（多行）
    pub(super) commit_input: Entity<InputState>,
    /// 是否 amend 上一次提交（默认 false）
    pub(super) commit_amend: bool,
    /// 是否 GPG 签名 commit（默认 false；用户切 toggle 后保持状态）
    pub(super) commit_sign: bool,
    /// 切仓库后待恢复的 commit 草稿；Render 内 cx.defer_in 调 set_value 写回 InputState
    pub(super) pending_commit_text: Option<SharedString>,
    /// 当前选中查看 diff 的文件（path + 来源分组）
    pub(super) selected_file: Option<(String, GroupKind)>,
    /// 当前文件的 diff 快照
    pub(super) current_diff: Option<FileDiff>,
    /// diff 是否正在拉取中
    pub(super) loading_diff: bool,
    /// 视图模式：工作区 / 历史
    pub(super) view_mode: ViewMode,
    /// History 累积的 commit 列表（按页 append）
    pub(super) history_commits: Vec<Commit>,
    /// History 是否还可能有下一页（上次拉满 PAGE_SIZE 即认为有）
    pub(super) history_has_more: bool,
    /// History 是否正在拉取中
    pub(super) loading_history: bool,
    /// Stash 列表
    pub(super) stashes: Vec<Stash>,
    /// Stash 是否正在拉取中
    pub(super) loading_stashes: bool,
    /// 新建分支输入框
    pub(super) create_branch_input: Entity<InputState>,
    /// 新建分支的源（None=当前 HEAD；Some(name)=指定分支作 base）
    pub(super) create_branch_base: Option<String>,
    /// Tag 列表（按 git for-each-ref 顺序）
    pub(super) tags: Vec<Tag>,
    /// Tag 是否正在拉取
    pub(super) loading_tags: bool,
    /// 新建 tag 输入框：tag 名
    pub(super) create_tag_input: Entity<InputState>,
    /// 新建 tag 输入框：message（可选；非空 → annotated tag，空 → lightweight）
    pub(super) create_tag_message_input: Entity<InputState>,
    /// sidebar 「本地分支」段是否折叠（默认展开）
    pub(super) collapsed_local: bool,
    /// sidebar 「远程分支」段是否折叠（默认折叠，远程通常较多）
    pub(super) collapsed_remote: bool,
    /// sidebar 「Tag」段是否折叠（默认折叠，tag 通常较多）
    pub(super) collapsed_tag: bool,
    /// 当前 diff 中被勾选准备 stage 的 (hunk_index, line_index_in_hunk)
    pub(super) selected_diff_lines: std::collections::HashSet<(usize, usize)>,
    /// 用户已点击展开的 diff spacer：(hunk_idx, run_start_line_idx)；切换文件 / commit 时清空
    pub(super) expanded_diff_spacers: std::collections::HashSet<(usize, usize)>,
    /// 远程仓库列表（git remote -v 解析）
    pub(super) remotes: Vec<Remote>,
    /// remote 列表是否正在拉取
    pub(super) loading_remotes: bool,
    /// 当前在 commit 详情视图查看的 commit（None = 处于 history 列表态）
    pub(super) viewing_commit: Option<Commit>,
    /// 详情视图的文件列表（git diff-tree --name-status 解析）
    pub(super) commit_files: Vec<FileStatus>,
    /// 详情视图当前选中查看 diff 的文件
    pub(super) selected_commit_file: Option<String>,
    /// 详情视图当前文件的 diff 快照
    pub(super) commit_file_diff: Option<FileDiff>,
    /// 详情视图文件列表是否正在拉取
    pub(super) loading_commit_files: bool,
    /// commit 详情 / Changes 文件树折叠目录（分开维护：commit 切换时只清前者）
    pub(super) commit_files_collapsed: std::collections::HashSet<String>,
    pub(super) changes_collapsed_dirs: std::collections::HashSet<String>,
    /// 单文件历史过滤路径（None = 全仓库 history；Some(path) = 仅该文件）
    pub(super) history_path_filter: Option<String>,
    /// commit 搜索关键词（按 message grep / author / since 解析）
    pub(super) history_search_input: Entity<InputState>,
    /// blame 行列表（当前 selected_file 的）
    pub(super) blame_lines: Vec<ramag_domain::entities::BlameLine>,
    pub(super) loading_blame: bool,
    /// diff header 切换：false=显示 diff（默认）/ true=显示 blame
    pub(super) showing_blame: bool,
    /// 行号 inline blame：Some = 顶部 banner 显示该行作者；点行号触发，× 关闭
    pub(super) inline_blame_text: Option<SharedString>,
    /// diff 忽略空白，对应 git diff -w
    pub(super) diff_ignore_whitespace: bool,
    /// diff 视图模式：标准（默认带 3 行上下文）/ 全文件 / 仅变更（前端过滤 Context）
    pub(super) diff_view_mode: DiffViewMode,
    /// reflog 条目列表
    pub(super) reflog_entries: Vec<ramag_domain::entities::ReflogEntry>,
    /// reflog 是否正在拉取
    pub(super) loading_reflog: bool,
    /// history 顶部切换：false=commit 列表（默认）/ true=reflog 列表
    pub(super) showing_reflog: bool,
    /// IDE 布局：上半区左右拖拽（左 files / 右 main）
    pub(super) ide_left_resize: Entity<ResizableState>,
    /// IDE 布局：上半 / 下半（history pane）之间的垂直拖拽
    pub(super) ide_files_resize: Entity<ResizableState>,
    /// IDE 布局：下半 history pane 右半内部 middle / commit detail 拖拽
    pub(super) detail_resize: Entity<ResizableState>,
    /// 顶层视图：仓库管理页 / 进入了仓库的 session
    pub(super) active_view: ActiveView,
    /// 最近打开仓库（启动从 storage.list_repos 加载，打开/删除时单条 upsert/delete）
    pub(super) recent_repos: Vec<RepoConfig>,
    /// 仓库管理页搜索框
    pub(super) repo_search_input: Entity<InputState>,
    /// IDE 左侧 Files panel 当前显示模式（Changes / Project / Stash）
    pub(super) files_view_mode: FilesViewMode,
    /// IDE 左侧 Files panel 文件搜索框（按 path substring 过滤当前 mode 列表）
    pub(super) files_search_input: Entity<InputState>,
    /// Project Files 视图：仓库内所有 tracked + untracked 但未 ignore 的相对路径（按字母排序）
    pub(super) project_files: Vec<String>,
    /// Project Files 是否正在拉取
    pub(super) loading_project_files: bool,
    /// Project Files 已展开目录（相对路径），默认空集合 = 全部折叠
    pub(super) project_expanded_dirs: std::collections::HashSet<String>,
    /// 缓存版本号：reload / toggle / expand_all / collapse_all 时递增对应字段；
    /// render 用 (files_version, expanded_version, query) 比对 cache 命中，
    /// 命中即跳过 build_tree + flatten，从 O(N log N) 降到 Rc clone
    pub(super) project_files_version: u64,
    pub(super) project_expanded_dirs_version: u64,
    pub(super) project_rows_cache: RefCell<Option<ProjectRowsCacheEntry>>,
    /// Project Files 虚拟列表滚动句柄（uniform_list 行级虚拟化，与 dbclient 表树同款）
    pub(super) project_scroll: UniformListScrollHandle,
    /// Project Files 模式当前选中查看内容的文件路径（与 selected_file 互独立：
    /// 前者展示**文件内容**，后者展示 diff，避免两个视图模式互相干扰）
    pub(super) selected_pf_path: Option<String>,
    /// Project Files 当前选中文件的内容快照（None = 未加载 / 未选中）
    pub(super) current_file_content: Option<FileContentSnapshot>,
    /// 文件内容是否正在读盘
    pub(super) loading_file_content: bool,
    /// Project Files 文件内容渲染的虚拟列表滚动句柄（垂直方向，uniform_list 行级虚拟化）
    pub(super) pf_content_scroll: UniformListScrollHandle,
    /// Diff 视图的虚拟化列表滚动 handle（unified / split 共用一个）
    pub(super) diff_scroll: UniformListScrollHandle,
    /// commit 文件列表 / 冲突编辑器滚动
    pub(super) commit_files_scroll: UniformListScrollHandle,
    pub(super) conflict_ours_scroll: UniformListScrollHandle,
    pub(super) conflict_theirs_scroll: UniformListScrollHandle,
    /// 虚拟列表滚动句柄：history 中栏 + reflog 列表（uniform_list 行级，万级也不卡）
    pub(super) history_scroll: UniformListScrollHandle,
    pub(super) reflog_scroll: UniformListScrollHandle,
    /// pf_content / diff 横向滚动句柄：uniform_list 管 Y，外层 overflow_x_scroll 管 X
    pub(super) pf_content_h_scroll: ScrollHandle,
    /// unified diff + split 模式左栏 横滚 handle
    pub(super) diff_h_scroll: ScrollHandle,
    /// split 模式右栏独立横滚 handle（IDEA 风格：左右两栏长行各自横滚不互相牵连）
    pub(super) diff_h_scroll_right: ScrollHandle,
    /// 下半区 history pane 是否显示（默认隐藏，工具栏 PanelBottom 图标 toggle）
    pub(super) history_pane_visible: bool,

    // ---- 多仓库 Tabs ----
    pub(super) open_repos: Vec<RepoConfig>,
    pub(super) file_tabs: Vec<FileTab>,
    pub(super) active_file_tab_idx: Option<usize>,
    pub(super) repo_session_cache: std::collections::HashMap<String, RepoSessionState>,

    // ---- Clone 对话框 ----
    pub(super) clone_url_input: Entity<InputState>,
    pub(super) clone_dest_path: Option<PathBuf>,
    pub(super) show_clone_panel: bool,

    // ---- Interactive Rebase ----
    pub(super) show_rebase_plan: bool,
    pub(super) rebase_plan_onto: String,
    pub(super) rebase_todos: Vec<RebaseTodo>,
    pub(super) loading_rebase_plan: bool,

    // ---- Conflict Editor ----
    pub(super) conflict_editor_path: Option<String>,
    pub(super) conflict_content: Option<ConflictContent>,
    pub(super) loading_conflict: bool,

    /// 视图焦点（cmd-w / 全局 action dispatch）
    pub(super) focus_handle: FocusHandle,
}

impl EventEmitter<VcsEvent> for VcsView {}

impl Focusable for VcsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl VcsView {
    /// 切换 IDE 左侧 Files panel 的视图模式（Changes / Project / Stash）
    ///
    /// 切到 Project 模式时若列表还没加载，触发一次异步拉取
    pub(super) fn set_files_view_mode(&mut self, mode: FilesViewMode, cx: &mut Context<Self>) {
        if self.files_view_mode != mode {
            self.files_view_mode = mode;
            // 切 mode 时清掉「另一边」的选中态，避免主区残留旧视图
            // - 离开 Project：清 selected_pf_path / current_file_content
            // - 离开 Changes：清 selected_file / current_diff
            if !matches!(mode, FilesViewMode::Project) {
                self.selected_pf_path = None;
                self.current_file_content = None;
                self.loading_file_content = false;
            } else {
                self.selected_file = None;
                self.current_diff = None;
                self.loading_diff = false;
            }
            cx.notify();
            // 切到任何 mode 都立即异步 reload 对应数据（实时更新，不需要刷新按钮）
            // - Changes: status；Project: ls-files；Stash: stash list；Branches: 分支列表
            self.refresh_current_files_view(cx);
        }
    }

    /// 切到仓库管理页（保留当前 repo 数据，仅切视图）
    pub(super) fn show_repo_list(&mut self, cx: &mut Context<Self>) {
        self.active_view = ActiveView::RepoList;
        cx.notify();
    }

    /// 清空所有 session 派生数据（diff / pf 内容 / commit 详情 / 历史 / 文件 tabs 等）
    /// open_repo_async 里切仓库时调用，确保新仓库不残留旧仓库的视图状态
    pub(super) fn clear_session_data(&mut self) {
        self.selected_file = None;
        self.current_diff = None;
        self.loading_diff = false;
        self.selected_pf_path = None;
        self.current_file_content = None;
        self.loading_file_content = false;
        self.viewing_commit = None;
        self.commit_files.clear();
        self.commit_files_collapsed.clear();
        self.selected_commit_file = None;
        self.commit_file_diff = None;
        self.loading_commit_files = false;
        self.show_rebase_plan = false;
        self.rebase_todos.clear();
        self.conflict_editor_path = None;
        self.conflict_content = None;
        self.history_commits.clear();
        self.history_has_more = false;
        self.project_files.clear();
        self.file_tabs.clear();
        self.active_file_tab_idx = None;
    }

    /// 首次打开 lazy load 首页 commits，避免仓库打开就预拉 git log
    pub(super) fn toggle_history_pane(&mut self, cx: &mut Context<Self>) {
        self.history_pane_visible = !self.history_pane_visible;
        if self.history_pane_visible
            && self.history_commits.is_empty()
            && !self.loading_history
            && self.repo.is_some()
        {
            self.load_history_page(0, cx);
        }
        cx.notify();
    }

    /// 清除当前错误（关闭顶部错误 banner 时调用）
    pub(super) fn clear_error(&mut self, cx: &mut Context<Self>) {
        if self.error.is_some() {
            self.error = None;
            cx.notify();
        }
    }

    /// 切换 diff 视图模式；FullFile 与 Standard 后端 unified 行数不同，要清缓存重拉
    pub(super) fn set_diff_view_mode(&mut self, mode: DiffViewMode, cx: &mut Context<Self>) {
        if self.diff_view_mode == mode {
            return;
        }
        let need_refetch = self.diff_view_mode.context_lines() != mode.context_lines();
        self.diff_view_mode = mode;
        if need_refetch {
            self.invalidate_active_diff_and_refetch(cx);
        } else {
            cx.notify();
        }
    }

    /// 清当前 active tab 的 diff 缓存 + 触发重拉（视 source 调对应 select_*）
    fn invalidate_active_diff_and_refetch(&mut self, cx: &mut Context<Self>) {
        if let Some(idx) = self.active_file_tab_idx
            && let Some(tab) = self.file_tabs.get_mut(idx)
        {
            tab.cached_diff = None;
        }
        self.current_diff = None;
        if let Some((p, k)) = self.selected_file.clone() {
            self.select_file(p, k, cx);
        } else if let Some(commit) = self.viewing_commit.clone()
            && let Some(path) = self.selected_commit_file.clone()
        {
            self.select_commit_file(path, commit.id.0, cx);
        } else {
            cx.notify();
        }
    }
}
