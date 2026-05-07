//! Git 领域实体：纯数据结构 + serde。infra 层（gix）填值，UI 层只读

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 仓库运行时 UUID（不持久化进 git；上层用 path 去重）
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

/// 仓库配置：本地路径 + 别名 + UI 偏好
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    pub id: RepoId,
    pub name: String,
    /// 工作树根路径（含 .git 的那一级）
    pub path: String,
    /// 上次打开时间，用于「最近」排序
    pub last_opened_at: Option<chrono::DateTime<chrono::Utc>>,
    /// 置顶展示
    #[serde(default)]
    pub favorite: bool,
}

impl RepoConfig {
    /// name 取路径最后一段
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

/// 文件变更类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
    /// 重命名：path 是新名，old_path 持旧名
    Renamed,
    Copied,
    /// 普通文件 ↔ 软链接
    TypeChanged,
    Untracked,
    /// merge / rebase 进行中
    Conflicted,
}

/// 单文件的工作区 + 暂存区状态。同一文件可同时 staged + unstaged（先 add 再改）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStatus {
    /// 工作树相对路径
    pub path: String,
    /// rename / copy 时的旧路径
    pub old_path: Option<String>,
    /// 暂存区相对 HEAD 的变更，None = 暂存区无此改动
    pub staged: Option<FileChangeKind>,
    /// 工作区相对暂存区的变更，None = 与暂存区一致
    pub unstaged: Option<FileChangeKind>,
}

impl FileStatus {
    pub fn is_conflicted(&self) -> bool {
        matches!(self.staged, Some(FileChangeKind::Conflicted))
            || matches!(self.unstaged, Some(FileChangeKind::Conflicted))
    }
}

/// 工作区状态聚合
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkingTreeStatus {
    /// 当前 HEAD 指向的分支，detached 时 None
    pub head_branch: Option<String>,
    /// HEAD 短 hash
    pub head_commit: Option<String>,
    /// merge / rebase / cherry-pick 进行中
    pub operation: Option<RepoOperation>,
    /// 全部文件（staged + unstaged + untracked + conflicted）
    pub files: Vec<FileStatus>,
    /// 落后远程的 commit 数
    pub behind: Option<usize>,
    /// 领先远程的 commit 数
    pub ahead: Option<usize>,
}

/// 仓库进行中的特殊操作
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepoOperation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

/// `git reset` 模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResetKind {
    /// 移动 HEAD，暂存区 / 工作区不动
    Soft,
    /// 移动 HEAD + 重置暂存区，工作区不动（默认）
    Mixed,
    /// 重置 HEAD + 暂存区 + 工作区（危险，会丢未提交改动）
    Hard,
}

/// 提交 OID（40 位 hex）
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CommitId(pub String);

impl CommitId {
    /// 短 hash（前 7 位）
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

/// 作者 / 提交者签名
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signature {
    pub name: String,
    pub email: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// commit 对象（log 列表 + 详情共用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    pub id: CommitId,
    /// merge 时多个父
    pub parents: Vec<CommitId>,
    pub author: Signature,
    pub committer: Signature,
    /// 消息首行
    pub subject: String,
    /// 首行之后的正文，可能为空
    pub body: String,
    /// 关联的 ref 名（branch / remote / tag），用于 log 贴标签
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

/// `git log` 查询参数
#[derive(Debug, Clone, Default)]
pub struct LogOptions {
    /// 起点，None = HEAD
    pub start: Option<String>,
    /// 单文件历史过滤
    pub path_filter: Option<String>,
    /// 分页跳过条数
    pub skip: usize,
    /// 取条数，None = 全部（UI 通常按页 100）
    pub limit: Option<usize>,
    /// 仅当前分支可达（false = 所有可达）
    pub current_branch_only: bool,
    /// `--grep=`：按 message 关键词过滤
    pub grep: Option<String>,
    /// `--author=`：可填名字或邮箱片段
    pub author: Option<String>,
    /// `--since=`：git 自然时间，如 "1 week ago"
    pub since: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BranchKind {
    Local,
    Remote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    /// 短名，不含 `refs/heads/` / `refs/remotes/<remote>/` 前缀
    pub name: String,
    pub kind: BranchKind,
    /// tip 指向的 commit
    pub commit: CommitId,
    pub is_head: bool,
    /// 上游分支（如 `origin/main`），仅 Local 有意义
    pub upstream: Option<String>,
    /// 领先 upstream 的 commit 数（仅 Local）
    pub ahead: Option<usize>,
    /// 落后 upstream 的 commit 数（仅 Local）
    pub behind: Option<usize>,
}

/// diff 行类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffLineKind {
    Context,
    Add,
    Delete,
}

/// 一行 diff 内容
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    /// 旧文件行号，None = 新增行
    pub old_lineno: Option<u32>,
    /// 新文件行号，None = 删除行
    pub new_lineno: Option<u32>,
    /// 行内容，不含 `+/-/<space>` 前缀
    pub text: String,
}

/// 一个 hunk
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

/// 单文件完整 diff
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub change_kind: FileChangeKind,
    /// 二进制文件不渲染 hunks
    pub binary: bool,
    pub old_mode: Option<u32>,
    pub new_mode: Option<u32>,
    pub hunks: Vec<Hunk>,
}

/// Reflog 单条：HEAD@{N} 时刻的 ref 状态 + 操作。用于「找回丢失 commit」
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflogEntry {
    /// 该时刻 ref 指向的 commit
    pub commit: CommitId,
    /// 形如 "HEAD@{0}"
    pub selector: String,
    /// commit / checkout / reset / merge / rebase ...
    pub action: String,
    /// reflog message
    pub subject: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// blame 单行：文件某一行的归属
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlameLine {
    pub commit: CommitId,
    pub author: String,
    /// author time
    pub timestamp: chrono::DateTime<chrono::Utc>,
    /// 当前文件中的行号（1-based）
    pub line_no: u32,
    /// 该 commit 的 subject
    pub subject: String,
    /// 该行原始内容
    pub content: String,
}

/// 交互式 rebase 单 commit 的处置动作
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

/// 交互式 rebase 单条计划
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebaseTodo {
    pub action: RebaseAction,
    /// 完整 commit hash
    pub hash: String,
    /// 首行消息
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

/// 三方冲突文件内容
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictContent {
    pub path: String,
    /// stage 2：HEAD 侧
    pub ours: Vec<String>,
    /// stage 3：MERGE_HEAD 侧
    pub theirs: Vec<String>,
    /// stage 1：共同祖先
    pub base: Vec<String>,
}

/// diff 来源对比
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffKind {
    /// 工作区 vs 暂存区
    WorkingTreeVsIndex,
    /// 暂存区 vs HEAD
    IndexVsHead,
    /// 工作区 vs HEAD
    WorkingTreeVsHead,
    /// commit vs 父
    CommitVsParent(CommitId),
    /// 任意两 commit 之间
    Range { from: CommitId, to: CommitId },
}

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
    /// 仅 annotated tag 有
    pub message: Option<String>,
    pub tagger: Option<Signature>,
}
