//! VCS view 多个子模块共享的类型 + 独立辅助函数
//!
//! 拆分自最初的 vcs_view.rs（>1300 行）。本 mod 为兄弟子模块（workspace_panel /
//! history_panel）提供：
//! - 视图状态枚举（ViewMode / FileOp / RemoteOp / GroupKind）
//! - 行尾按钮 / 文件状态字母 / 颜色映射
//! - History commit 行渲染（拆到 [`commit_row`] 子模块）

mod commit_row;

pub(super) use commit_row::render_commit_row;

use gpui::{AnyElement, ClickEvent, Context, IntoElement, SharedString};
use gpui_component::{
    Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
};
use ramag_domain::entities::{FileChangeKind, FileDiff};

use super::vcs_view::VcsView;

// 重导出，让外部 import 不感知 graph 文件位置
pub(super) use super::commit_graph::build_commit_lanes;

/// 主视图当前展示模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ViewMode {
    /// 工作区（变更 / commit / 分支）
    Workspace,
    /// 历史日志
    History,
}

/// VcsView 顶层激活视图（仿 dbclient_view::CenterMode）
///
/// - `RepoList`：仓库管理页（最近列表 + [+] 选择新仓库）
/// - `Session`：进入仓库的 IDE 布局
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ActiveView {
    RepoList,
    Session,
}

/// IDE 左侧 Files panel 的视图模式（仿 IDEA Git Tool Window 切换）
///
/// 顶部以一排 segmented icon 按钮切换；默认 [`Project`] 让用户开仓库就能看到完整目录。
///
/// - `Project`（默认）：完整项目目录树（带 git 状态标记，点击查看文件内容）
/// - `Changes`：仅显示有变更的文件（已暂存 / 未暂存 / 未跟踪 / 冲突分组，点击查看 diff）
/// - `Stash`：当前仓库的 stash 列表
/// - `Branches`：本地 / 远程分支列表（原右侧 sidebar 的分支段，可独立 tab 切入）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FilesViewMode {
    Project,
    Changes,
    Stash,
}

impl FilesViewMode {
    /// 用于 tooltip 的中文标签
    pub(super) fn label(self) -> &'static str {
        match self {
            FilesViewMode::Project => "项目文件",
            FilesViewMode::Changes => "本地变更",
            FilesViewMode::Stash => "暂存堆栈",
        }
    }

    /// 用于 tab 按钮的 dom id 后缀
    pub(super) fn id_str(self) -> &'static str {
        match self {
            FilesViewMode::Project => "project",
            FilesViewMode::Changes => "changes",
            FilesViewMode::Stash => "stash",
        }
    }
}

/// History 面板每页加载条数
pub(super) const HISTORY_PAGE_SIZE: usize = 100;

/// Diff 视图二态：[`Standard`]=带少量上下文（git -U3，默认）/ [`FullFile`]=展示全文件（-U999999）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DiffViewMode {
    Standard,
    FullFile,
}

impl DiffViewMode {
    /// 后端 unified 上下文行数：3=标准；999999=全文件
    pub(super) fn context_lines(self) -> u32 {
        match self {
            DiffViewMode::Standard => 3,
            DiffViewMode::FullFile => 999_999,
        }
    }

    /// 切换：标准 ↔ 全文件
    pub(super) fn toggled(self) -> Self {
        match self {
            DiffViewMode::Standard => DiffViewMode::FullFile,
            DiffViewMode::FullFile => DiffViewMode::Standard,
        }
    }
}

/// 文件级写操作种类（行尾按钮触发）
#[derive(Debug, Clone, Copy)]
pub(super) enum FileOp {
    Stage,
    Unstage,
    Discard,
}

/// 远程同步操作种类（顶部工具栏按钮触发）
#[derive(Debug, Clone, Copy)]
pub(super) enum RemoteOp {
    Fetch,
    Pull,
    /// 普通 push（force=false）
    Push,
    /// 安全强推（git push --force-with-lease）—— 用于改写历史后推送（rebase / amend）
    PushForce,
}

/// 文件分组所属（决定行尾按钮的 stage/unstage/discard 组合）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GroupKind {
    Staged,
    Unstaged,
    Untracked,
    Conflict,
}

/// Project Files 视图点击文件后加载到的内容快照
///
/// 渲染层只读这些字段，不再访问磁盘。读盘 / 截断 / 二进制判断 / max_chars 计算
/// 都在 [`super::super::vcs_view_ops_repo::file_io`] 异步路径内一次性完成。
///
/// `lines` 用 `Rc<Vec<String>>` 持有，让 render 层 clone 是引用计数（O(1)），
/// 避免每帧整文件拷贝（4MB 文件可省几十 MB 内存搬运）。
#[derive(Clone)]
pub(super) struct FileContentSnapshot {
    /// 仓库根的相对路径（与 ls-files 输出一致）
    pub path: String,
    /// 按行拆分后的内容；二进制 / 读失败时为空
    pub lines: std::rc::Rc<Vec<String>>,
    /// 最长行字符数（select_pf_file 时算一次缓存，render 直接读，省 100 万次 chars()）
    pub max_chars: usize,
    /// 是否被 4MB 阈值截断（true 时 lines 仅含前 N 行）
    pub truncated: bool,
    /// 是否被识别为二进制（前 8KB 含 NUL 字节即视为二进制）
    pub binary: bool,
    /// 读盘失败时的错误描述（None = 成功）
    pub error: Option<String>,
}

/// 文件 tab 来源：Changes 走工作区 diff / ProjectFiles 走文件内容 / Commit 走 commit diff
///
/// 三类来源共用同一套 file_tab + 主区渲染路径，避免主区出现「左 Changes 点 vs 右下 commit 点」
/// 各走一条路的不一致。点击触发各自的 fetch 后写入 tab.cached_diff / cached_content。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum FileTabSource {
    Changes(GroupKind),
    ProjectFiles,
    /// commit_id：完整 hash；change_kind：Modified/Added/Deleted/Renamed/...（决定状态字母）
    Commit {
        commit_id: String,
        change_kind: Option<FileChangeKind>,
    },
}

/// 主区已打开的文件 tab（统一服务 Changes diff 和 ProjectFiles 内容）
#[derive(Clone)]
pub(super) struct FileTab {
    pub path: String,
    pub source: FileTabSource,
    /// Changes 来源拉到的 diff（ProjectFiles 始终 None）
    pub cached_diff: Option<FileDiff>,
    /// ProjectFiles 来源读到的文件内容快照（Changes 始终 None）
    pub cached_content: Option<FileContentSnapshot>,
}

/// Stash 行尾按钮触发的操作
#[derive(Debug, Clone, Copy)]
pub(super) enum StashOp {
    /// 应用某个 stash（不删）
    Apply(usize),
    /// 应用某个 stash 后删除
    Pop(usize),
    /// 仅删除某个 stash
    Drop(usize),
}

/// 分支操作（checkout / create / delete / merge / rebase）
#[derive(Debug, Clone)]
pub(super) enum BranchOp {
    Checkout(String),
    /// (name, base) — base=None 从 HEAD 创建；创建后会自动 checkout 到新分支
    Create(String, Option<String>),
    /// (name, force) — force=true 用 -D 强制删未合并分支
    Delete(String, bool),
    /// 把指定分支合并到当前 HEAD（默认 --no-ff，强制建 merge commit）
    Merge(String),
    /// 把当前 HEAD rebase 到指定分支上（git rebase &lt;name&gt;）
    Rebase(String),
}

/// 冲突文件解决操作（行尾按钮触发）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConflictOp {
    /// 采纳「我们」（HEAD 侧）的版本
    UseOurs,
    /// 采纳「他们」（对方分支）的版本
    UseTheirs,
    /// 单纯标记为已解决（用户手改后调）= git add
    MarkResolved,
}

/// 进行中操作的「继续 / 中止 / 跳过」按钮触发
///
/// `Skip` 仅 rebase 支持（merge / cherry-pick 时按钮置灰）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OperationStep {
    Continue,
    Abort,
    Skip,
}

/// Tag 操作（创建 / 删除 / 推送）
#[derive(Debug, Clone)]
pub(super) enum TagOp {
    /// (name, message=None 表示 lightweight；Some 创建 annotated；target=None 基于 HEAD)
    Create {
        name: String,
        message: Option<String>,
    },
    /// 删除本地 tag
    Delete(String),
    /// 推送 tag 到 origin
    Push(String),
}

/// 行尾操作小按钮：触发 self.run_file_op（已转图标按钮 + tooltip）
pub(super) fn file_op_button(
    id_parts: (&'static str, usize, &str),
    label: &'static str,
    op: FileOp,
    path: String,
    busy: bool,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let id = SharedString::from(format!("vcs-{}-{}-{}", id_parts.0, id_parts.1, id_parts.2));
    let mut btn = Button::new(id)
        .ghost()
        .xsmall()
        .tooltip(label)
        .disabled(busy);
    btn = match op {
        FileOp::Stage => btn.icon(IconName::Plus),
        FileOp::Unstage => btn.icon(IconName::Minus),
        FileOp::Discard => btn.icon(ramag_ui::icons::trash()),
    };
    btn.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
        this.confirm_file_op(op, path.clone(), window, cx);
    }))
    .into_any_element()
}

/// 文件状态字母（M / A / D / R / C / T / ? / U）
pub(super) fn code_to_letter(kind: Option<FileChangeKind>) -> &'static str {
    match kind {
        Some(FileChangeKind::Modified) => "M",
        Some(FileChangeKind::Added) => "A",
        Some(FileChangeKind::Deleted) => "D",
        Some(FileChangeKind::Renamed) => "R",
        Some(FileChangeKind::Copied) => "C",
        Some(FileChangeKind::TypeChanged) => "T",
        Some(FileChangeKind::Untracked) => "?",
        Some(FileChangeKind::Conflicted) => "U",
        None => " ",
    }
}

/// 不同变更类型用不同颜色（M 暖橙 / A 绿 / D 红 / R 蓝 / U 深红）
pub(super) fn code_letter_color(code: &str, fallback: gpui::Hsla) -> gpui::Hsla {
    match code {
        "M" => gpui::hsla(40.0 / 360.0, 0.7, 0.55, 1.0),
        "A" => gpui::hsla(140.0 / 360.0, 0.55, 0.45, 1.0),
        "D" => gpui::hsla(0.0, 0.65, 0.55, 1.0),
        "R" => gpui::hsla(220.0 / 360.0, 0.6, 0.55, 1.0),
        "C" => gpui::hsla(220.0 / 360.0, 0.6, 0.55, 1.0),
        "T" => gpui::hsla(280.0 / 360.0, 0.55, 0.55, 1.0),
        "U" => gpui::hsla(0.0, 0.75, 0.5, 1.0),
        _ => fallback,
    }
}
