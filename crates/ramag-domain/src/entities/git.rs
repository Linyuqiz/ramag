//! Git 相关领域实体
//!
//! 与底层实现无关：纯数据结构 + serde。infra 层（gix 实现）将这些结构填好返回。
//! UI 层（tool-vcs）只读取这些结构渲染。
//!
//! 实体一览：
//! - 仓库：[`RepoId`] / [`RepoConfig`]
//! - 工作区：[`WorkingTreeStatus`] / [`FileStatus`] / [`FileChangeKind`]
//! - 提交：[`Commit`] / [`CommitId`] / [`Signature`]
//! - 分支：[`Branch`] / [`BranchKind`]
//! - 差异：[`FileDiff`] / [`Hunk`] / [`DiffLine`] / [`DiffLineKind`] / [`DiffKind`]
//! - Stash：[`Stash`] / [`StashId`]
//! - 远程 / Tag：[`Remote`] / [`Tag`]

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// =============================================================================
// 仓库
// =============================================================================

/// 仓库唯一标识符（运行时 UUID，不持久化到 git 仓库本身）
///
/// 用于 ramag 自己的连接缓存 / 池索引。同一物理仓库每次"打开"会产生新 RepoId，
/// 但相同 path 的两次同时打开应该共用——上层用 path 去重判定
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RepoId(pub Uuid);

impl RepoId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for RepoId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for RepoId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 仓库配置：本地路径 + 用户起的别名 + UI 偏好
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    pub id: RepoId,
    /// 用户起的名字（默认取目录名）
    pub name: String,
    /// 仓库工作树根路径（含 .git 子目录的那一级）
    pub path: String,
    /// 上次打开时间（用于"最近"排序）
    pub last_opened_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 收藏标记（置顶展示）
    #[serde(default)]
    pub favorite: bool,
}

impl RepoConfig {
    /// 从路径快速构造（name 取最后一段目录名）
    pub fn from_path(path: impl Into<String>) -> Self {
        let path: String = path.into();
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        Self {
            id: RepoId::new(),
            name,
            path,
            last_opened_at: None,
            favorite: false,
        }
    }
}

// =============================================================================
// 工作区状态
// =============================================================================

/// 一个文件相对工作区的变更类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileChangeKind {
    /// 新增（git add 后）
    Added,
    /// 修改
    Modified,
    /// 删除
    Deleted,
    /// 重命名（path 字段是新名，old_path 字段持旧名）
    Renamed,
    /// 拷贝（少见）
    Copied,
    /// 类型变更（普通文件 ↔ 软链接 等）
    TypeChanged,
    /// 未跟踪（never staged）
    Untracked,
    /// 冲突（merge / rebase 进行中）
    Conflicted,
}

/// 单个文件的工作区 / 暂存区状态
///
/// 同一个文件可能同时既在 staged 又在 unstaged（先 add 再继续改）；
/// 调用方按 [`FileStatus::staged`] / [`FileStatus::unstaged`] 字段区分两种状态各自的 kind
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStatus {
    /// 工作树相对路径
    pub path: String,
    /// rename / copy 时的旧路径
    pub old_path: Option<String>,
    /// 暂存区与 HEAD 的差异类型（None = 此文件没在暂存区有改动）
    pub staged: Option<FileChangeKind>,
    /// 工作区与暂存区的差异类型（None = 工作区与暂存区一致）
    pub unstaged: Option<FileChangeKind>,
}

impl FileStatus {
    /// 是否冲突状态（merge / rebase 中）
    pub fn is_conflicted(&self) -> bool {
        matches!(self.staged, Some(FileChangeKind::Conflicted))
            || matches!(self.unstaged, Some(FileChangeKind::Conflicted))
    }
}

/// 整个工作区的状态聚合
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkingTreeStatus {
    /// 当前 HEAD 指向的分支名（detached 时为 None）
    pub head_branch: Option<String>,
    /// 当前 HEAD 的 commit id（短 hash）
    pub head_commit: Option<String>,
    /// 是否在 merge / rebase / cherry-pick 进行中
    pub operation: Option<RepoOperation>,
    /// 所有文件状态（含 staged + unstaged + untracked + conflicted）
    pub files: Vec<FileStatus>,
    /// 当前分支落后远程的 commit 数
    pub behind: Option<usize>,
    /// 当前分支领先远程的 commit 数（未 push）
    pub ahead: Option<usize>,
}

/// 仓库正在进行的特殊操作（影响 commit / checkout 的可用性）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepoOperation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

/// `git reset` 三种模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResetKind {
    /// `--soft`：移动 HEAD，暂存区 / 工作区不动
    Soft,
    /// `--mixed`（默认）：移动 HEAD + 重置暂存区，工作区不动
    Mixed,
    /// `--hard`：移动 HEAD + 重置暂存区 + 重置工作区（危险，会丢未提交改动）
    Hard,
}

// =============================================================================
// 提交
// =============================================================================

/// 提交对象 ID（git OID 全 40 位 hex）
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CommitId(pub String);

impl CommitId {
    /// 短 hash（前 7 位），UI 展示用
    pub fn short(&self) -> &str {
        if self.0.len() > 7 {
            &self.0[..7]
        } else {
            &self.0
        }
    }
}

impl std::fmt::Display for CommitId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// 提交者 / 作者签名
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature {
    pub name: String,
    pub email: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// 一个 commit 对象（log 列表项 + 详情共用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    pub id: CommitId,
    /// 父 commit（merge 时多个）
    pub parents: Vec<CommitId>,
    pub author: Signature,
    pub committer: Signature,
    /// 第一行（subject）
    pub subject: String,
    /// 第一行之后的部分（body，可能为空）
    pub body: String,
    /// 该 commit 关联的 ref 名（local branch / remote / tag），UI 用于在 log 里贴标签
    #[serde(default)]
    pub refs: Vec<String>,
}

impl Commit {
    pub fn message_full(&self) -> String {
        if self.body.is_empty() {
            self.subject.clone()
        } else {
            format!("{}\n\n{}", self.subject, self.body)
        }
    }
}

/// `log` 查询参数
#[derive(Debug, Clone, Default)]
pub struct LogOptions {
    /// 起点（None = HEAD）
    pub start: Option<String>,
    /// 单文件历史的过滤路径
    pub path_filter: Option<String>,
    /// 跳过前 N 条（分页）
    pub skip: usize,
    /// 取多少条（None = 全部，谨慎；UI 通常按页 100）
    pub limit: Option<usize>,
    /// 仅当前分支（默认 false = 所有 reachable）
    pub current_branch_only: bool,
    /// 按 commit message 关键词过滤（git log --grep=）
    pub grep: Option<String>,
    /// 按作者过滤（git log --author=；可以是名字或邮箱片段）
    pub author: Option<String>,
    /// 时间下限（git log --since=；接受 "1 week ago" / "2024-01-01" 等 git 自然时间）
    pub since: Option<String>,
}

// =============================================================================
// 分支
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BranchKind {
    Local,
    Remote,
}

/// 分支
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    /// 短名（不含 `refs/heads/` / `refs/remotes/origin/` 前缀）
    pub name: String,
    pub kind: BranchKind,
    /// 此分支 tip 指向的 commit
    pub commit: CommitId,
    /// 当前 HEAD 是否就是这个分支
    pub is_head: bool,
    /// 跟踪的上游分支（如 `origin/main`），仅 Local 有意义
    pub upstream: Option<String>,
    /// 领先 upstream 的 commit 数（仅 Local 有意义）
    pub ahead: Option<usize>,
    /// 落后 upstream 的 commit 数（仅 Local 有意义）
    pub behind: Option<usize>,
}

// =============================================================================
// Diff
// =============================================================================

/// 一行 diff 的类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffLineKind {
    /// 上下文（未变）
    Context,
    /// 增（绿色 `+`）
    Add,
    /// 删（红色 `-`）
    Delete,
}

/// 一行 diff 内容
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    /// 旧文件行号（None = 这是新增行）
    pub old_lineno: Option<u32>,
    /// 新文件行号（None = 这是删除行）
    pub new_lineno: Option<u32>,
    /// 行内容（不含开头的 `+/-/<space>` 标识符）
    pub text: String,
}

/// 一个 hunk（@@ -old_start,old_lines +new_start,new_lines @@）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    /// hunk 头注释（如函数名）
    pub heading: Option<String>,
    pub lines: Vec<DiffLine>,
}

/// 单文件的完整 diff
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub change_kind: FileChangeKind,
    /// 是否二进制（不渲染 hunks）
    pub binary: bool,
    /// 文件级别 mode 变更（旧 mode / 新 mode）
    pub old_mode: Option<u32>,
    pub new_mode: Option<u32>,
    pub hunks: Vec<Hunk>,
}

/// Reflog 单条：HEAD@{N} 时刻的 ref 状态 + 操作类型 + 描述
///
/// 用于「找回丢失 commit」场景：reset --hard 后看 reflog，把 HEAD 切回任意条目即可
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflogEntry {
    /// 该时刻 ref 指向的 commit
    pub commit: CommitId,
    /// reflog selector（如 "HEAD@{0}"）
    pub selector: String,
    /// 操作类型（commit / checkout / reset / merge / rebase 等）
    pub action: String,
    /// 操作描述（reflog message，例如 "checkout: moving from main to feature"）
    pub subject: String,
    /// 操作时间
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// blame 单行：指向引入此行的 commit + 作者 + 时间 + 该行内容
///
/// 与 [`Commit`] 不同——这里是「文件某一行的归属」，每行带 commit 元数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlameLine {
    /// 引入此行的 commit
    pub commit: CommitId,
    /// 引入此行的作者名
    pub author: String,
    /// 引入此行的时间（author time）
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// 当前文件中的行号（1-based）
    pub line_no: u32,
    /// 该 commit 的 subject（首行简短）
    pub subject: String,
    /// 该行原始内容（去掉 git blame 的 `\t` 前缀）
    pub content: String,
}

// =============================================================================
// Interactive Rebase
// =============================================================================

/// 交互式 rebase 中每个 commit 的处置动作
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RebaseAction {
    Pick,
    Squash,
    Fixup,
    Reword,
    Edit,
    Drop,
}

impl RebaseAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pick => "pick",
            Self::Squash => "squash",
            Self::Fixup => "fixup",
            Self::Reword => "reword",
            Self::Edit => "edit",
            Self::Drop => "drop",
        }
    }

    pub fn label_zh(self) -> &'static str {
        match self {
            Self::Pick => "保留",
            Self::Squash => "合并+保留消息",
            Self::Fixup => "合并+丢弃消息",
            Self::Reword => "修改消息",
            Self::Edit => "暂停修改",
            Self::Drop => "删除",
        }
    }
}

/// Interactive rebase 单条计划项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebaseTodo {
    pub action: RebaseAction,
    /// 完整 commit hash
    pub hash: String,
    /// subject（首行消息）
    pub subject: String,
}

impl RebaseTodo {
    pub fn short_hash(&self) -> &str {
        if self.hash.len() > 7 {
            &self.hash[..7]
        } else {
            &self.hash
        }
    }
}

// =============================================================================
// Conflict Editor
// =============================================================================

/// 三方冲突文件内容（ours = HEAD 侧，theirs = MERGE_HEAD 侧，base = 共同祖先）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictContent {
    pub path: String,
    /// stage 2：HEAD 版本（我们的修改）
    pub ours: Vec<String>,
    /// stage 3：MERGE_HEAD 版本（对方的修改）
    pub theirs: Vec<String>,
    /// stage 1：共同祖先版本（merge base）
    pub base: Vec<String>,
}

/// 取 diff 的来源对比
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffKind {
    /// 工作区 vs 暂存区（"unstaged"）
    WorkingTreeVsIndex,
    /// 暂存区 vs HEAD（"staged"）
    IndexVsHead,
    /// 工作区 vs HEAD（合并视图）
    WorkingTreeVsHead,
    /// 某个 commit vs 其父
    CommitVsParent(CommitId),
    /// 任意两个 commit 之间
    Range { from: CommitId, to: CommitId },
}

// =============================================================================
// Stash / Remote / Tag
// =============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StashId(pub usize);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stash {
    pub id: StashId,
    pub message: String,
    pub commit: CommitId,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Remote {
    pub name: String,
    pub fetch_url: String,
    pub push_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TagKind {
    Lightweight,
    Annotated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub kind: TagKind,
    pub commit: CommitId,
    /// annotated tag 才有
    pub message: Option<String>,
    pub tagger: Option<Signature>,
}
