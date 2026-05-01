//! Ramag Git driver
//!
//! 实现 [`ramag_domain::traits::GitDriver`]，底层用 [`gix`]（gitoxide，纯 Rust）。
//!
//! # 设计要点
//!
//! - **同步 → async 桥接**：gix 主要是同步 API，本 crate 的 [`runtime`] 模块用
//!   `std::thread + futures::oneshot` 把同步调用派发到独立线程，结果用 oneshot 送回，
//!   让 GPUI 异步任务能 await。**不需要 tokio runtime**（与 `ramag-infra-storage` 同款模式）
//! - **仓库句柄缓存**：按 [`RepoId`] 索引缓存 [`gix::Repository`] 句柄，避免每次操作
//!   重新打开仓库（gix 打开有开销，含读 .git/config、refs 索引等）
//! - **错误映射**：把 gix 各模块的错误聚合成 [`ramag_domain::error::DomainError`]，
//!   附带中文消息便于 UI 展示

pub mod blame;
pub mod cherry_pick;
pub mod clone;
pub mod commit_files;
pub mod commit_op;
pub mod conflict_content;
pub mod conflict_ops;
pub mod diff;
pub mod errors;
pub mod git_cmd;
pub mod history_ops;
pub mod log;
pub mod merge;
pub mod patch;
pub mod rebase;
pub mod rebase_interactive;
pub mod reflog;
pub mod remote;
pub mod runtime;
pub mod stash;
pub mod status;
pub mod tag;
pub mod work_ops;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::Mutex;

use ramag_domain::entities::{
    BlameLine, Branch, BranchKind, Commit, CommitId, ConflictContent, DiffKind, FileDiff,
    FileStatus, LogOptions, RebaseTodo, ReflogEntry, Remote, RepoConfig, RepoId, ResetKind, Stash,
    Tag, WorkingTreeStatus,
};
use ramag_domain::error::{DomainError, Result};
use ramag_domain::traits::GitDriver;

use crate::runtime::run_blocking;

/// 已打开仓库的内部句柄
///
/// gix::Repository 不是 Sync，必须包 Mutex；clone 是 Arc 引用计数 +1（O(1)）
struct OpenRepo {
    repo: Arc<Mutex<gix::Repository>>,
    path: PathBuf,
    /// 写操作串行化锁：所有写 git index 的 op（stage/unstage/discard/commit/checkout
    /// /branch/stash/tag/merge/cherry_pick/reset/revert/rebase/patch/remote_admin）
    /// 都在 worker 线程内先 lock 再跑，避免并发触发 `.git/index.lock` 冲突
    write_lock: Arc<Mutex<()>>,
}

/// Git 驱动主结构
#[derive(Clone, Default)]
pub struct GitDriverImpl {
    /// path → OpenRepo（按物理路径去重，避免同一仓库被多次打开）
    by_path: Arc<DashMap<PathBuf, RepoId>>,
    /// RepoId → OpenRepo
    repos: Arc<DashMap<RepoId, Arc<OpenRepo>>>,
}

impl GitDriverImpl {
    pub fn new() -> Self {
        Self::default()
    }

    /// 内部：取已打开仓库句柄；没打开就报错
    fn get_repo(&self, id: &RepoId) -> Result<Arc<OpenRepo>> {
        self.repos
            .get(id)
            .map(|r| r.clone())
            .ok_or_else(|| DomainError::InvalidConfig(format!("仓库未打开: {id}")))
    }
}

/// 写操作 helper：worker 线程内先 lock 写锁，再执行 git 命令
///
/// 闭包接收 repo path 而不是整个 handle，让调用方写起来短小。所有写 git index 的方法
/// 都该走这个，以避免并发触发 `.git/index.lock` 冲突。
async fn run_write_blocking<F, T>(handle: Arc<OpenRepo>, f: F) -> Result<T>
where
    F: FnOnce(&Path) -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    run_blocking(move || {
        let _g = handle.write_lock.lock();
        f(&handle.path)
    })
    .await
}

#[async_trait]
impl GitDriver for GitDriverImpl {
    fn name(&self) -> &'static str {
        "gix"
    }

    async fn open_repo(&self, path: &Path) -> Result<RepoConfig> {
        let canonical = path
            .canonicalize()
            .map_err(|e| DomainError::InvalidConfig(format!("路径无法访问: {e}")))?;

        // 同 path 已打开过 → 复用
        if let Some(existing_id) = self.by_path.get(&canonical) {
            let id = existing_id.clone();
            drop(existing_id);
            let path_string = canonical.to_string_lossy().into_owned();
            return Ok(RepoConfig::from_path(path_string).with_id(id));
        }

        // 新仓库：在阻塞线程里打开（gix 打开有 I/O，避免阻塞 GPUI 异步线程）
        let canonical_for_open = canonical.clone();
        let repo = run_blocking(move || {
            gix::open(&canonical_for_open).map_err(crate::errors::map_open_error)
        })
        .await?;

        let id = RepoId::new();
        let handle = Arc::new(OpenRepo {
            repo: Arc::new(Mutex::new(repo)),
            path: canonical.clone(),
            write_lock: Arc::new(Mutex::new(())),
        });
        self.repos.insert(id.clone(), handle);
        self.by_path.insert(canonical.clone(), id.clone());

        let path_string = canonical.to_string_lossy().into_owned();
        Ok(RepoConfig::from_path(path_string).with_id(id))
    }

    async fn close_repo(&self, repo: &RepoId) -> Result<()> {
        if let Some((_, handle)) = self.repos.remove(repo) {
            self.by_path.remove(&handle.path);
        }
        Ok(())
    }

    async fn status(&self, repo: &RepoId) -> Result<WorkingTreeStatus> {
        let handle = self.get_repo(repo)?;
        run_blocking(move || {
            let path = handle.path.clone();
            let repo = handle.repo.lock();
            crate::status::collect_status(&repo, &path)
        })
        .await
    }

    async fn list_branches(&self, repo: &RepoId, kind: BranchKind) -> Result<Vec<Branch>> {
        let handle = self.get_repo(repo)?;
        run_blocking(move || {
            let repo = handle.repo.lock();
            crate::status::list_branches(&repo, kind)
        })
        .await
    }

    async fn log(&self, repo: &RepoId, opts: LogOptions) -> Result<Vec<Commit>> {
        let handle = self.get_repo(repo)?;
        run_blocking(move || crate::log::run_log(&handle.path, &opts)).await
    }

    async fn diff_file(&self, repo: &RepoId, path: &str, kind: DiffKind) -> Result<FileDiff> {
        let handle = self.get_repo(repo)?;
        let path = path.to_string();
        run_blocking(move || crate::diff::run_diff(&handle.path, &path, &kind)).await
    }

    async fn diff_file_opts(
        &self,
        repo: &RepoId,
        path: &str,
        kind: DiffKind,
        ignore_whitespace: bool,
    ) -> Result<FileDiff> {
        let handle = self.get_repo(repo)?;
        let path = path.to_string();
        run_blocking(move || {
            crate::diff::run_diff_opts(&handle.path, &path, &kind, ignore_whitespace)
        })
        .await
    }

    async fn diff_file_full_opts(
        &self,
        repo: &RepoId,
        path: &str,
        kind: DiffKind,
        ignore_whitespace: bool,
        context_lines: u32,
    ) -> Result<FileDiff> {
        let handle = self.get_repo(repo)?;
        let path = path.to_string();
        run_blocking(move || {
            crate::diff::run_diff_full_opts(
                &handle.path,
                &path,
                &kind,
                ignore_whitespace,
                context_lines,
            )
        })
        .await
    }

    // ---- Phase B 写操作：subprocess git 兜底（gix 写 API 还在演进期）----

    async fn stage(&self, repo: &RepoId, paths: &[String]) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let paths = paths.to_vec();
        run_write_blocking(handle, move |p| crate::work_ops::stage(p, &paths)).await
    }

    async fn unstage(&self, repo: &RepoId, paths: &[String]) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let paths = paths.to_vec();
        run_write_blocking(handle, move |p| crate::work_ops::unstage(p, &paths)).await
    }

    async fn discard(&self, repo: &RepoId, paths: &[String]) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let paths = paths.to_vec();
        run_write_blocking(handle, move |p| crate::work_ops::discard(p, &paths)).await
    }

    async fn list_files(&self, repo: &RepoId) -> Result<Vec<String>> {
        let handle = self.get_repo(repo)?;
        run_blocking(move || crate::work_ops::list_files(&handle.path)).await
    }

    async fn commit(
        &self,
        repo: &RepoId,
        message: &str,
        amend: bool,
        sign: bool,
    ) -> Result<CommitId> {
        let handle = self.get_repo(repo)?;
        let message = message.to_string();
        run_write_blocking(handle, move |p| {
            crate::commit_op::run(p, &message, amend, sign)
        })
        .await
    }

    // ---- Phase C：分支操作 ----

    async fn checkout(&self, repo: &RepoId, target: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let target = target.to_string();
        run_write_blocking(handle, move |p| crate::work_ops::checkout(p, &target)).await
    }

    async fn create_branch(&self, repo: &RepoId, name: &str, base: Option<&str>) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let name = name.to_string();
        let base = base.map(str::to_owned);
        run_write_blocking(handle, move |p| {
            crate::work_ops::create_branch(p, &name, base.as_deref())
        })
        .await
    }

    async fn delete_branch(&self, repo: &RepoId, name: &str, force: bool) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let name = name.to_string();
        run_write_blocking(handle, move |p| {
            crate::work_ops::delete_branch(p, &name, force)
        })
        .await
    }

    // ---- Phase C：远程同步 ----

    async fn fetch(&self, repo: &RepoId, remote: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let remote = remote.to_string();
        run_write_blocking(handle, move |p| crate::remote::fetch(p, &remote)).await
    }

    async fn push(
        &self,
        repo: &RepoId,
        remote: &str,
        branch: &str,
        set_upstream: bool,
        force_with_lease: bool,
    ) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let remote = remote.to_string();
        let branch = branch.to_string();
        run_write_blocking(handle, move |p| {
            crate::remote::push(p, &remote, &branch, set_upstream, force_with_lease)
        })
        .await
    }

    async fn pull(&self, repo: &RepoId, remote: &str, branch: &str, rebase: bool) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let remote = remote.to_string();
        let branch = branch.to_string();
        run_write_blocking(handle, move |p| {
            crate::remote::pull(p, &remote, &branch, rebase)
        })
        .await
    }

    // ---- Phase C：Stash 操作 ----

    async fn list_stashes(&self, repo: &RepoId) -> Result<Vec<Stash>> {
        let handle = self.get_repo(repo)?;
        run_blocking(move || crate::stash::list(&handle.path)).await
    }

    async fn stash_save(
        &self,
        repo: &RepoId,
        message: Option<&str>,
        include_untracked: bool,
    ) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let msg = message.map(str::to_owned);
        run_write_blocking(handle, move |p| {
            crate::stash::save(p, msg.as_deref(), include_untracked)
        })
        .await
    }

    async fn stash_apply(&self, repo: &RepoId, idx: usize, pop: bool) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, move |p| crate::stash::apply(p, idx, pop)).await
    }

    async fn stash_drop(&self, repo: &RepoId, idx: usize) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, move |p| crate::stash::drop(p, idx)).await
    }

    // ---- Phase D：Tag 操作 ----

    async fn list_tags(&self, repo: &RepoId) -> Result<Vec<Tag>> {
        let handle = self.get_repo(repo)?;
        run_blocking(move || crate::tag::list(&handle.path)).await
    }

    async fn create_tag(
        &self,
        repo: &RepoId,
        name: &str,
        target: Option<&str>,
        message: Option<&str>,
        sign: bool,
    ) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let name = name.to_string();
        let target = target.map(str::to_owned);
        let message = message.map(str::to_owned);
        run_write_blocking(handle, move |p| {
            crate::tag::create(p, &name, target.as_deref(), message.as_deref(), sign)
        })
        .await
    }

    async fn delete_tag(&self, repo: &RepoId, name: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let name = name.to_string();
        run_write_blocking(handle, move |p| crate::tag::delete(p, &name)).await
    }

    async fn push_tag(&self, repo: &RepoId, remote: &str, name: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let remote = remote.to_string();
        let name = name.to_string();
        run_write_blocking(handle, move |p| crate::tag::push(p, &remote, &name)).await
    }

    // ---- Phase D：行级 patch apply ----

    async fn stage_patch(&self, repo: &RepoId, patch: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let patch = patch.to_string();
        run_write_blocking(handle, move |p| crate::patch::stage(p, &patch)).await
    }

    async fn unstage_patch(&self, repo: &RepoId, patch: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let patch = patch.to_string();
        run_write_blocking(handle, move |p| crate::patch::unstage(p, &patch)).await
    }

    async fn discard_patch(&self, repo: &RepoId, patch: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let patch = patch.to_string();
        run_write_blocking(handle, move |p| crate::patch::discard(p, &patch)).await
    }

    // ---- Phase D：合并 / Cherry-pick / 冲突解决 ----

    async fn merge(
        &self,
        repo: &RepoId,
        branch: &str,
        no_ff: bool,
        ff_only: bool,
        message: Option<&str>,
    ) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let branch = branch.to_string();
        let message = message.map(str::to_owned);
        run_write_blocking(handle, move |p| {
            crate::merge::start(p, &branch, no_ff, ff_only, message.as_deref())
        })
        .await
    }

    async fn merge_abort(&self, repo: &RepoId) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, crate::merge::abort).await
    }

    async fn merge_continue(&self, repo: &RepoId) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, crate::merge::cont).await
    }

    async fn cherry_pick(&self, repo: &RepoId, commit: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let commit = commit.to_string();
        run_write_blocking(handle, move |p| crate::cherry_pick::start(p, &commit)).await
    }

    async fn cherry_pick_abort(&self, repo: &RepoId) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, crate::cherry_pick::abort).await
    }

    async fn cherry_pick_continue(&self, repo: &RepoId) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, crate::cherry_pick::cont).await
    }

    async fn use_ours(&self, repo: &RepoId, paths: &[String]) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let paths = paths.to_vec();
        run_write_blocking(handle, move |p| crate::conflict_ops::use_ours(p, &paths)).await
    }

    async fn use_theirs(&self, repo: &RepoId, paths: &[String]) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let paths = paths.to_vec();
        run_write_blocking(handle, move |p| crate::conflict_ops::use_theirs(p, &paths)).await
    }

    // ---- Phase D：Reset / Revert / Rebase ----

    async fn reset(&self, repo: &RepoId, target: &str, kind: ResetKind) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let target = target.to_string();
        run_write_blocking(handle, move |p| crate::history_ops::reset(p, &target, kind)).await
    }

    async fn revert(&self, repo: &RepoId, commit: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let commit = commit.to_string();
        run_write_blocking(handle, move |p| crate::history_ops::revert(p, &commit)).await
    }

    async fn rebase(&self, repo: &RepoId, onto: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let onto = onto.to_string();
        run_write_blocking(handle, move |p| {
            crate::git_cmd::run_git_bytes(p, &["rebase", &onto]).map(|_| ())
        })
        .await
    }

    async fn rebase_continue(&self, repo: &RepoId) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, move |p| {
            crate::git_cmd::run_git_bytes(p, &["rebase", "--continue"]).map(|_| ())
        })
        .await
    }

    async fn rebase_skip(&self, repo: &RepoId) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, move |p| {
            crate::git_cmd::run_git_bytes(p, &["rebase", "--skip"]).map(|_| ())
        })
        .await
    }

    async fn rebase_abort(&self, repo: &RepoId) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, move |p| {
            crate::git_cmd::run_git_bytes(p, &["rebase", "--abort"]).map(|_| ())
        })
        .await
    }

    // ---- Phase D：Remote 管理 ----

    async fn list_remotes(&self, repo: &RepoId) -> Result<Vec<Remote>> {
        let handle = self.get_repo(repo)?;
        run_blocking(move || crate::remote::list(&handle.path)).await
    }

    async fn add_remote(&self, repo: &RepoId, name: &str, url: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let name = name.to_string();
        let url = url.to_string();
        run_write_blocking(handle, move |p| crate::remote::add(p, &name, &url)).await
    }

    async fn remove_remote(&self, repo: &RepoId, name: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let name = name.to_string();
        run_write_blocking(handle, move |p| crate::remote::remove(p, &name)).await
    }

    async fn set_remote_url(&self, repo: &RepoId, name: &str, url: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let name = name.to_string();
        let url = url.to_string();
        run_write_blocking(handle, move |p| crate::remote::set_url(p, &name, &url)).await
    }

    // ---- Phase D：Commit 详情 ----

    async fn list_commit_files(&self, repo: &RepoId, commit: &str) -> Result<Vec<FileStatus>> {
        let handle = self.get_repo(repo)?;
        let commit = commit.to_string();
        run_blocking(move || crate::commit_files::list(&handle.path, &commit)).await
    }

    async fn blame(&self, repo: &RepoId, path: &str) -> Result<Vec<BlameLine>> {
        let handle = self.get_repo(repo)?;
        let path = path.to_string();
        run_blocking(move || crate::blame::run(&handle.path, &path)).await
    }

    // ---- Phase E：Reflog ----

    async fn list_reflog(
        &self,
        repo: &RepoId,
        ref_name: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<ReflogEntry>> {
        let handle = self.get_repo(repo)?;
        let ref_name = ref_name.map(str::to_owned);
        run_blocking(move || crate::reflog::list(&handle.path, ref_name.as_deref(), limit)).await
    }

    // ---- Clone / Init ----

    async fn clone_repo(&self, url: &str, dest: &Path) -> Result<RepoConfig> {
        let url = url.to_string();
        let dest_clone = dest.to_path_buf();
        run_blocking(move || crate::clone::clone_repo(&url, &dest_clone)).await?;
        self.open_repo(dest).await
    }

    async fn init_repo(&self, path: &Path) -> Result<RepoConfig> {
        let path_init = path.to_path_buf();
        run_blocking(move || crate::clone::init_repo(&path_init)).await?;
        self.open_repo(path).await
    }

    // ---- Interactive Rebase ----

    async fn interactive_rebase_plan(&self, repo: &RepoId, onto: &str) -> Result<Vec<RebaseTodo>> {
        let handle = self.get_repo(repo)?;
        let onto = onto.to_string();
        run_blocking(move || crate::rebase_interactive::plan(&handle.path, &onto)).await
    }

    async fn interactive_rebase_execute(
        &self,
        repo: &RepoId,
        onto: &str,
        todos: &[RebaseTodo],
    ) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let onto = onto.to_string();
        let todos = todos.to_vec();
        run_write_blocking(handle, move |p| {
            crate::rebase_interactive::execute(p, &onto, &todos)
        })
        .await
    }

    async fn get_conflict_content(&self, repo: &RepoId, path: &str) -> Result<ConflictContent> {
        let handle = self.get_repo(repo)?;
        let path = path.to_string();
        run_blocking(move || crate::conflict_content::get_content(&handle.path, &path)).await
    }
}

/// 给 RepoConfig 提供从外部注入 RepoId 的便利构造方法
trait RepoConfigExt {
    fn with_id(self, id: RepoId) -> Self;
}

impl RepoConfigExt for RepoConfig {
    fn with_id(mut self, id: RepoId) -> Self {
        self.id = id;
        self
    }
}
