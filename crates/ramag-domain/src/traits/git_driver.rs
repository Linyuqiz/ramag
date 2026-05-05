//! GitDriver trait：Git 仓库操作统一抽象
//!
//! 与 [`crate::traits::Driver`]（SQL）/ [`crate::traits::KvDriver`]（KV）并列。
//!
//! # 设计要点
//!
//! 1. **dyn-safe**：方法不引入关联类型；底层实现（gix）按 `RepoId` 索引缓存仓库句柄
//! 2. **同步 → async 桥接**：gix 主要是同步 API，infra 层用 `std::thread + oneshot` 桥接
//!    让 GPUI 异步任务能 await（参考 `ramag-infra-storage` 同款模式）
//! 3. **路径而非句柄**：方法传 `&RepoId`，让上层保留稳定 ID；具体仓库句柄缓存在 driver 内部
//! 4. **写操作分批**：commit / push / merge 这种"会改 HEAD/refs"的方法都是单次 RPC，
//!    不暴露中间事务 / 锁，让 driver 内部串行化
//!
//! # 阶段
//!
//! Phase A（v0.1 骨架）：仅暴露 open_repo / status / list_branches / log / diff_file 等读操作
//! Phase B（v0.1 daily）：补 stage / unstage / commit / checkout / create_branch
//! Phase C（v0.1 远程）：补 fetch / push / pull / list_stashes / stash_*
//! Phase D+（v0.2+）：merge / rebase / cherry-pick / tag / blame ...

use std::path::Path;

use async_trait::async_trait;

use crate::entities::{
    BlameLine, Branch, BranchKind, Commit, ConflictContent, FileDiff, FileStatus, LogOptions,
    RebaseTodo, ReflogEntry, Remote, RepoConfig, RepoId, ResetKind, Stash, Tag, WorkingTreeStatus,
};
use crate::error::{DomainError, Result};

/// `NotImplemented` 默认实现统一短路：让 trait 默认方法体能压成单行调用，
/// 避免每个 stub 方法都重复 5 行 `Err(DomainError::NotImplemented(...))`
fn not_impl<T>(method: &'static str) -> Result<T> {
    Err(DomainError::NotImplemented(method.into()))
}

#[async_trait]
pub trait GitDriver: Send + Sync {
    /// 驱动名称（"gix" / "git2-fallback" 等）
    fn name(&self) -> &'static str;

    /// 打开本地仓库目录（含 `.git`），返回 driver 内部分配的 RepoId
    ///
    /// 调用方持 RepoId 后续读写；driver 内部按 ConnectionId 缓存仓库句柄
    async fn open_repo(&self, path: &Path) -> Result<RepoConfig>;

    /// 关闭仓库（释放底层句柄；保留 RepoConfig 在 ramag 自己的存储里）
    async fn close_repo(&self, repo: &RepoId) -> Result<()>;

    /// 工作区当前状态：HEAD / 变更文件列表 / ahead-behind / merge 进行中等
    async fn status(&self, repo: &RepoId) -> Result<WorkingTreeStatus>;

    /// 列出本地或远程分支
    async fn list_branches(&self, repo: &RepoId, kind: BranchKind) -> Result<Vec<Branch>>;

    /// 查询提交日志（流式分页：调用方按需多次调用调整 LogOptions::skip）
    async fn log(&self, repo: &RepoId, opts: LogOptions) -> Result<Vec<Commit>>;

    /// 单文件 diff
    ///
    /// `kind` 控制对比来源（工作区 vs 暂存区 / 暂存区 vs HEAD / commit vs 父）
    async fn diff_file(
        &self,
        repo: &RepoId,
        path: &str,
        kind: crate::entities::DiffKind,
    ) -> Result<FileDiff>;

    /// 单文件 diff（带 ignore_whitespace 开关；UI 的 [⎵] toggle 用）
    ///
    /// 默认实现退化到 `diff_file`，忽略 ignore_whitespace 参数。
    async fn diff_file_opts(
        &self,
        repo: &RepoId,
        path: &str,
        kind: crate::entities::DiffKind,
        _ignore_whitespace: bool,
    ) -> Result<FileDiff> {
        self.diff_file(repo, path, kind).await
    }

    /// 单文件 diff（含 ignore_whitespace + 自定义上下文行数）
    ///
    /// `context_lines` 控制 unified diff 上下文行数：
    /// - 3：标准（git diff 默认）
    /// - 999_999：等价「展示全文件」
    /// - 0：仅变更行（全靠后端去掉上下文）
    /// 默认实现忽略 `context_lines` 退化到 `diff_file_opts`
    async fn diff_file_full_opts(
        &self,
        repo: &RepoId,
        path: &str,
        kind: crate::entities::DiffKind,
        ignore_whitespace: bool,
        _context_lines: u32,
    ) -> Result<FileDiff> {
        self.diff_file_opts(repo, path, kind, ignore_whitespace)
            .await
    }

    // ---- 以下方法 Phase B+ 实现，先用默认 NotImplemented 占位 ----

    /// 把指定文件加入暂存区
    async fn stage(&self, _repo: &RepoId, _paths: &[String]) -> Result<()> {
        not_impl("stage")
    }

    /// 把指定文件从暂存区撤回
    async fn unstage(&self, _repo: &RepoId, _paths: &[String]) -> Result<()> {
        not_impl("unstage")
    }

    /// 丢弃工作区改动（git checkout -- <path>）
    async fn discard(&self, _repo: &RepoId, _paths: &[String]) -> Result<()> {
        not_impl("discard")
    }

    /// 创建 commit（amend=true 时修改上一次而不是新建；sign=true 时 GPG 签名）
    async fn commit(
        &self,
        _repo: &RepoId,
        _message: &str,
        _amend: bool,
        _sign: bool,
    ) -> Result<crate::entities::CommitId> {
        not_impl("commit")
    }

    /// 切换到分支 / commit / tag
    async fn checkout(&self, _repo: &RepoId, _target: &str) -> Result<()> {
        not_impl("checkout")
    }

    /// 创建本地分支（base=None 时基于当前 HEAD）
    async fn create_branch(&self, _repo: &RepoId, _name: &str, _base: Option<&str>) -> Result<()> {
        not_impl("create_branch")
    }

    /// 删除本地分支（force=true 才允许删未合并的）
    async fn delete_branch(&self, _repo: &RepoId, _name: &str, _force: bool) -> Result<()> {
        not_impl("delete_branch")
    }

    /// 拉取远程更新（不合并）
    async fn fetch(&self, _repo: &RepoId, _remote: &str) -> Result<()> {
        not_impl("fetch")
    }

    /// 推送到远程
    ///
    /// - `set_upstream=true` 加 `-u`（首次推送新分支用，同时设置 upstream）
    /// - `force_with_lease=true` 加 `--force-with-lease`（比 --force 安全的强推）
    async fn push(
        &self,
        _repo: &RepoId,
        _remote: &str,
        _branch: &str,
        _set_upstream: bool,
        _force_with_lease: bool,
    ) -> Result<()> {
        not_impl("push")
    }

    /// fetch + merge / rebase 当前分支
    async fn pull(
        &self,
        _repo: &RepoId,
        _remote: &str,
        _branch: &str,
        _rebase: bool,
    ) -> Result<()> {
        not_impl("pull")
    }

    /// 列出所有 stash
    async fn list_stashes(&self, _repo: &RepoId) -> Result<Vec<Stash>> {
        not_impl("list_stashes")
    }

    /// 列出仓库内所有「git 跟踪 + 未跟踪但未被 ignore」的相对路径
    ///
    /// 用于 IDE 左侧 Project Files 视图（完整目录树）
    /// 实现：等价于 `git ls-files --cached --others --exclude-standard -z`
    async fn list_files(&self, _repo: &RepoId) -> Result<Vec<String>> {
        not_impl("list_files")
    }

    /// 保存当前工作区到 stash
    async fn stash_save(
        &self,
        _repo: &RepoId,
        _message: Option<&str>,
        _include_untracked: bool,
    ) -> Result<()> {
        not_impl("stash_save")
    }

    /// 应用某个 stash（pop=true 应用后删除）
    async fn stash_apply(&self, _repo: &RepoId, _idx: usize, _pop: bool) -> Result<()> {
        not_impl("stash_apply")
    }

    /// 删除某个 stash
    async fn stash_drop(&self, _repo: &RepoId, _idx: usize) -> Result<()> {
        not_impl("stash_drop")
    }

    // ---- Phase D：Tag 操作 ----

    /// 列出仓库内所有 tag（轻量 + annotated）
    async fn list_tags(&self, _repo: &RepoId) -> Result<Vec<Tag>> {
        not_impl("list_tags")
    }

    /// 创建 tag
    ///
    /// - `target=None` 表示基于当前 HEAD
    /// - `message=Some(_)` 创建 annotated tag，否则 lightweight
    /// - `sign=true` 时 GPG 签名（隐含 annotated；message=None 时也强制 annotated）
    async fn create_tag(
        &self,
        _repo: &RepoId,
        _name: &str,
        _target: Option<&str>,
        _message: Option<&str>,
        _sign: bool,
    ) -> Result<()> {
        not_impl("create_tag")
    }

    /// 删除本地 tag
    async fn delete_tag(&self, _repo: &RepoId, _name: &str) -> Result<()> {
        not_impl("delete_tag")
    }

    /// 推送指定 tag 到远程
    async fn push_tag(&self, _repo: &RepoId, _remote: &str, _name: &str) -> Result<()> {
        not_impl("push_tag")
    }

    // ---- Phase D：行级 / 分块 stage（patch apply）----

    /// 把一段 unified diff patch 写入暂存区
    ///
    /// 调用方负责构造合法的 patch 文本（含 file header / hunk header / 行）。
    /// 实现内部用 `git apply --cached --recount -`，自动重算 line counts。
    async fn stage_patch(&self, _repo: &RepoId, _patch: &str) -> Result<()> {
        not_impl("stage_patch")
    }

    /// 反向：把一段 patch 从暂存区撤回（不影响工作区）
    async fn unstage_patch(&self, _repo: &RepoId, _patch: &str) -> Result<()> {
        not_impl("unstage_patch")
    }

    /// 把 patch 反向应用到工作区：hunk 级回滚到 HEAD（不通过暂存区）
    /// 用于 IDEA 风格 hunk「↶」按钮：仅回滚某段改动，保留其他改动
    async fn discard_patch(&self, _repo: &RepoId, _patch: &str) -> Result<()> {
        not_impl("discard_patch")
    }

    // ---- Phase D：合并 / Cherry-pick / 冲突解决 ----

    /// 把指定分支合并到当前 HEAD
    ///
    /// - `no_ff=true`：即使可以 fast-forward 也强制创建 merge commit
    /// - `ff_only=true`：要求必须能 fast-forward，否则失败（不创建 merge commit）
    /// - 二者互斥；都为 false 时走 git 默认（可 ff 就 ff，否则建 merge commit）
    /// - 出现冲突时本方法返回 Err，但仓库已进入 merge 进行中状态，由 status() 反映
    async fn merge(
        &self,
        _repo: &RepoId,
        _branch: &str,
        _no_ff: bool,
        _ff_only: bool,
        _message: Option<&str>,
    ) -> Result<()> {
        not_impl("merge")
    }

    /// 中止进行中的 merge（git merge --abort）
    async fn merge_abort(&self, _repo: &RepoId) -> Result<()> {
        not_impl("merge_abort")
    }

    /// 续接 merge（冲突解决完后；git merge --continue）
    async fn merge_continue(&self, _repo: &RepoId) -> Result<()> {
        not_impl("merge_continue")
    }

    /// 把单个 commit 拣选到当前 HEAD（git cherry-pick <id>）
    async fn cherry_pick(&self, _repo: &RepoId, _commit: &str) -> Result<()> {
        not_impl("cherry_pick")
    }

    /// 中止进行中的 cherry-pick
    async fn cherry_pick_abort(&self, _repo: &RepoId) -> Result<()> {
        not_impl("cherry_pick_abort")
    }

    /// 续接 cherry-pick（冲突解决完后）
    async fn cherry_pick_continue(&self, _repo: &RepoId) -> Result<()> {
        not_impl("cherry_pick_continue")
    }

    /// 冲突解决：采纳「我们」的版本（HEAD 侧）
    ///
    /// `git checkout --ours -- <paths>` + `git add <paths>`，一步搞定
    async fn use_ours(&self, _repo: &RepoId, _paths: &[String]) -> Result<()> {
        not_impl("use_ours")
    }

    /// 冲突解决：采纳「他们」的版本（对方分支侧）
    async fn use_theirs(&self, _repo: &RepoId, _paths: &[String]) -> Result<()> {
        not_impl("use_theirs")
    }

    // ---- Phase D：Reset / Revert / Rebase ----

    /// 重置 HEAD 到指定 commit
    ///
    /// `kind`：决定是否同时重置暂存区 / 工作区
    /// 慎用 Hard——会丢失工作区未提交改动；UI 应弹二次确认
    async fn reset(&self, _repo: &RepoId, _target: &str, _kind: ResetKind) -> Result<()> {
        not_impl("reset")
    }

    /// 生成一个反向 commit 撤销指定 commit（不改写历史，安全）
    async fn revert(&self, _repo: &RepoId, _commit: &str) -> Result<()> {
        not_impl("revert")
    }

    /// 把当前分支 rebase 到 onto（onto 通常是另一分支名 / commit）
    async fn rebase(&self, _repo: &RepoId, _onto: &str) -> Result<()> {
        not_impl("rebase")
    }

    /// 续接 rebase（冲突解决完后）
    async fn rebase_continue(&self, _repo: &RepoId) -> Result<()> {
        not_impl("rebase_continue")
    }

    /// 跳过当前 commit（rebase 时；丢弃此 commit 继续下一个）
    async fn rebase_skip(&self, _repo: &RepoId) -> Result<()> {
        not_impl("rebase_skip")
    }

    /// 中止 rebase（恢复到 rebase 开始前的状态）
    async fn rebase_abort(&self, _repo: &RepoId) -> Result<()> {
        not_impl("rebase_abort")
    }

    // ---- Phase D：Remote 管理 ----

    /// 列出本仓库配置的所有 remote
    async fn list_remotes(&self, _repo: &RepoId) -> Result<Vec<Remote>> {
        not_impl("list_remotes")
    }

    /// 添加新 remote（git remote add <name> <url>）
    async fn add_remote(&self, _repo: &RepoId, _name: &str, _url: &str) -> Result<()> {
        not_impl("add_remote")
    }

    /// 删除 remote（git remote remove <name>）
    async fn remove_remote(&self, _repo: &RepoId, _name: &str) -> Result<()> {
        not_impl("remove_remote")
    }

    /// 修改 remote 的 fetch URL（push URL 仍走 fetch URL，简化场景）
    async fn set_remote_url(&self, _repo: &RepoId, _name: &str, _url: &str) -> Result<()> {
        not_impl("set_remote_url")
    }

    // ---- Phase D：Commit 详情 ----

    /// 列出指定 commit 引入的文件变更
    ///
    /// 返回的 [`FileStatus::staged`] 字段承载该 commit 的变更类型；
    /// `unstaged` 始终为 None（commit 已落地，无工作区/暂存区概念）
    async fn list_commit_files(&self, _repo: &RepoId, _commit: &str) -> Result<Vec<FileStatus>> {
        not_impl("list_commit_files")
    }

    /// 取指定文件的 blame（每行最后改人 + 时间 + commit subject）
    ///
    /// 返回结果按文件当前行号 1-based 顺序，长度等于文件总行数
    async fn blame(&self, _repo: &RepoId, _path: &str) -> Result<Vec<BlameLine>> {
        not_impl("blame")
    }

    // ---- Phase E：Reflog ----

    /// 列出指定 ref 的 reflog 条目（默认 HEAD）
    ///
    /// `ref_name=None` 等价于 HEAD；`limit=None` 让 git 用默认值
    async fn list_reflog(
        &self,
        _repo: &RepoId,
        _ref_name: Option<&str>,
        _limit: Option<usize>,
    ) -> Result<Vec<ReflogEntry>> {
        not_impl("list_reflog")
    }

    // ---- Clone / Init ----

    /// Clone 远程仓库到本地目录（`dest` 必须不存在或为空目录）
    async fn clone_repo(&self, _url: &str, _dest: &Path) -> Result<RepoConfig> {
        not_impl("clone_repo")
    }

    /// 在已有目录初始化新 git 仓库（`git init`）
    async fn init_repo(&self, _path: &Path) -> Result<RepoConfig> {
        not_impl("init_repo")
    }

    // ---- Interactive Rebase ----

    /// 取得 interactive rebase 的初始计划（`onto..HEAD` 的 commit 列表，全部标 Pick）
    async fn interactive_rebase_plan(
        &self,
        _repo: &RepoId,
        _onto: &str,
    ) -> Result<Vec<RebaseTodo>> {
        not_impl("interactive_rebase_plan")
    }

    /// 执行 interactive rebase：把用户编辑后的 todos 写成 todo 文件，再调 `git rebase -i`
    async fn interactive_rebase_execute(
        &self,
        _repo: &RepoId,
        _onto: &str,
        _todos: &[RebaseTodo],
    ) -> Result<()> {
        not_impl("interactive_rebase_execute")
    }

    // ---- Conflict Content ----

    /// 取冲突文件的三方内容（ours = stage 2，theirs = stage 3，base = stage 1）
    async fn get_conflict_content(&self, _repo: &RepoId, _path: &str) -> Result<ConflictContent> {
        not_impl("get_conflict_content")
    }
}
