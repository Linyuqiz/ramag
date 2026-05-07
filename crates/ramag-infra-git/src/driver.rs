//! `impl GitDriver`：thin wrapper。模式：取 handle → 参数 owned 化 → run_blocking / run_write_blocking

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;

use ramag_domain::entities::{
    BlameLine, Branch, BranchKind, Commit, CommitId, ConflictContent, DiffKind, FileDiff,
    FileStatus, LogOptions, RebaseTodo, ReflogEntry, Remote, RepoConfig, RepoId, ResetKind, Stash,
    Tag, WorkingTreeStatus,
};
use ramag_domain::error::{DomainError, Result};
use ramag_domain::traits::GitDriver;

use crate::handle::{OpenRepo, run_write_blocking};
use crate::runtime::run_blocking;
use crate::{
    GitDriverImpl, RepoConfigExt, blame, cherry_pick, clone, commit_files, commit_op,
    conflict_content, conflict_ops, diff, errors, git_cmd, history_ops, log, merge, patch,
    rebase_interactive, reflog, remote, stash, status, tag, work_ops,
};

#[async_trait]
impl GitDriver for GitDriverImpl {
    fn name(&self) -> &'static str {
        "gix"
    }

    async fn open_repo(&self, path: &Path) -> Result<RepoConfig> {
        let canonical = path
            .canonicalize()
            .map_err(|e| DomainError::InvalidConfig(format!("路径无法访问: {e}")))?;

        // 同 path 复用已打开句柄
        if let Some(existing_id) = self.by_path.get(&canonical) {
            let id = existing_id.clone();
            drop(existing_id);
            let path_string = canonical.to_string_lossy().into_owned();
            return Ok(RepoConfig::from_path(path_string).with_id(id));
        }

        // 阻塞线程打开（gix 有 I/O）
        let canonical_for_open = canonical.clone();
        let repo =
            run_blocking(move || gix::open(&canonical_for_open).map_err(errors::map_open_error))
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
            status::collect_status(&repo, &path)
        })
        .await
    }

    async fn list_branches(&self, repo: &RepoId, kind: BranchKind) -> Result<Vec<Branch>> {
        let handle = self.get_repo(repo)?;
        run_blocking(move || {
            let repo = handle.repo.lock();
            status::list_branches(&repo, kind)
        })
        .await
    }

    async fn log(&self, repo: &RepoId, opts: LogOptions) -> Result<Vec<Commit>> {
        let handle = self.get_repo(repo)?;
        run_blocking(move || log::run_log(&handle.path, &opts)).await
    }

    async fn diff_file(&self, repo: &RepoId, path: &str, kind: DiffKind) -> Result<FileDiff> {
        let handle = self.get_repo(repo)?;
        let path = path.to_string();
        run_blocking(move || diff::run_diff(&handle.path, &path, &kind)).await
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
        run_blocking(move || diff::run_diff_opts(&handle.path, &path, &kind, ignore_whitespace))
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
            diff::run_diff_full_opts(&handle.path, &path, &kind, ignore_whitespace, context_lines)
        })
        .await
    }

    // 写操作走 subprocess git（gix 写 API 还在演进）

    async fn stage(&self, repo: &RepoId, paths: &[String]) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let paths = paths.to_vec();
        run_write_blocking(handle, move |p| work_ops::stage(p, &paths)).await
    }

    async fn unstage(&self, repo: &RepoId, paths: &[String]) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let paths = paths.to_vec();
        run_write_blocking(handle, move |p| work_ops::unstage(p, &paths)).await
    }

    async fn discard(&self, repo: &RepoId, paths: &[String]) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let paths = paths.to_vec();
        run_write_blocking(handle, move |p| work_ops::discard(p, &paths)).await
    }

    async fn list_files(&self, repo: &RepoId) -> Result<Vec<String>> {
        let handle = self.get_repo(repo)?;
        run_blocking(move || work_ops::list_files(&handle.path)).await
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
        run_write_blocking(handle, move |p| commit_op::run(p, &message, amend, sign)).await
    }

    async fn checkout(&self, repo: &RepoId, target: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let target = target.to_string();
        run_write_blocking(handle, move |p| work_ops::checkout(p, &target)).await
    }

    async fn create_branch(&self, repo: &RepoId, name: &str, base: Option<&str>) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let name = name.to_string();
        let base = base.map(str::to_owned);
        run_write_blocking(handle, move |p| {
            work_ops::create_branch(p, &name, base.as_deref())
        })
        .await
    }

    async fn delete_branch(&self, repo: &RepoId, name: &str, force: bool) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let name = name.to_string();
        run_write_blocking(handle, move |p| work_ops::delete_branch(p, &name, force)).await
    }

    async fn fetch(&self, repo: &RepoId, remote: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let remote = remote.to_string();
        run_write_blocking(handle, move |p| remote::fetch(p, &remote)).await
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
            remote::push(p, &remote, &branch, set_upstream, force_with_lease)
        })
        .await
    }

    async fn pull(&self, repo: &RepoId, remote: &str, branch: &str, rebase: bool) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let remote = remote.to_string();
        let branch = branch.to_string();
        run_write_blocking(handle, move |p| remote::pull(p, &remote, &branch, rebase)).await
    }

    async fn list_stashes(&self, repo: &RepoId) -> Result<Vec<Stash>> {
        let handle = self.get_repo(repo)?;
        run_blocking(move || stash::list(&handle.path)).await
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
            stash::save(p, msg.as_deref(), include_untracked)
        })
        .await
    }

    async fn stash_apply(&self, repo: &RepoId, idx: usize, pop: bool) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, move |p| stash::apply(p, idx, pop)).await
    }

    async fn stash_drop(&self, repo: &RepoId, idx: usize) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, move |p| stash::drop(p, idx)).await
    }

    async fn list_tags(&self, repo: &RepoId) -> Result<Vec<Tag>> {
        let handle = self.get_repo(repo)?;
        run_blocking(move || tag::list(&handle.path)).await
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
            tag::create(p, &name, target.as_deref(), message.as_deref(), sign)
        })
        .await
    }

    async fn delete_tag(&self, repo: &RepoId, name: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let name = name.to_string();
        run_write_blocking(handle, move |p| tag::delete(p, &name)).await
    }

    async fn push_tag(&self, repo: &RepoId, remote: &str, name: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let remote = remote.to_string();
        let name = name.to_string();
        run_write_blocking(handle, move |p| tag::push(p, &remote, &name)).await
    }

    async fn stage_patch(&self, repo: &RepoId, patch: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let patch = patch.to_string();
        run_write_blocking(handle, move |p| patch::stage(p, &patch)).await
    }

    async fn unstage_patch(&self, repo: &RepoId, patch: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let patch = patch.to_string();
        run_write_blocking(handle, move |p| patch::unstage(p, &patch)).await
    }

    async fn discard_patch(&self, repo: &RepoId, patch: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let patch = patch.to_string();
        run_write_blocking(handle, move |p| patch::discard(p, &patch)).await
    }

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
            merge::start(p, &branch, no_ff, ff_only, message.as_deref())
        })
        .await
    }

    async fn merge_abort(&self, repo: &RepoId) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, merge::abort).await
    }

    async fn merge_continue(&self, repo: &RepoId) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, merge::cont).await
    }

    async fn cherry_pick(&self, repo: &RepoId, commit: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let commit = commit.to_string();
        run_write_blocking(handle, move |p| cherry_pick::start(p, &commit)).await
    }

    async fn cherry_pick_abort(&self, repo: &RepoId) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, cherry_pick::abort).await
    }

    async fn cherry_pick_continue(&self, repo: &RepoId) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, cherry_pick::cont).await
    }

    async fn use_ours(&self, repo: &RepoId, paths: &[String]) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let paths = paths.to_vec();
        run_write_blocking(handle, move |p| conflict_ops::use_ours(p, &paths)).await
    }

    async fn use_theirs(&self, repo: &RepoId, paths: &[String]) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let paths = paths.to_vec();
        run_write_blocking(handle, move |p| conflict_ops::use_theirs(p, &paths)).await
    }

    async fn reset(&self, repo: &RepoId, target: &str, kind: ResetKind) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let target = target.to_string();
        run_write_blocking(handle, move |p| history_ops::reset(p, &target, kind)).await
    }

    async fn revert(&self, repo: &RepoId, commit: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let commit = commit.to_string();
        run_write_blocking(handle, move |p| history_ops::revert(p, &commit)).await
    }

    async fn rebase(&self, repo: &RepoId, onto: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let onto = onto.to_string();
        run_write_blocking(handle, move |p| {
            git_cmd::run_git_bytes(p, &["rebase", &onto]).map(|_| ())
        })
        .await
    }

    async fn rebase_continue(&self, repo: &RepoId) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, move |p| {
            git_cmd::run_git_bytes(p, &["rebase", "--continue"]).map(|_| ())
        })
        .await
    }

    async fn rebase_skip(&self, repo: &RepoId) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, move |p| {
            git_cmd::run_git_bytes(p, &["rebase", "--skip"]).map(|_| ())
        })
        .await
    }

    async fn rebase_abort(&self, repo: &RepoId) -> Result<()> {
        let handle = self.get_repo(repo)?;
        run_write_blocking(handle, move |p| {
            git_cmd::run_git_bytes(p, &["rebase", "--abort"]).map(|_| ())
        })
        .await
    }

    async fn list_remotes(&self, repo: &RepoId) -> Result<Vec<Remote>> {
        let handle = self.get_repo(repo)?;
        run_blocking(move || remote::list(&handle.path)).await
    }

    async fn add_remote(&self, repo: &RepoId, name: &str, url: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let name = name.to_string();
        let url = url.to_string();
        run_write_blocking(handle, move |p| remote::add(p, &name, &url)).await
    }

    async fn remove_remote(&self, repo: &RepoId, name: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let name = name.to_string();
        run_write_blocking(handle, move |p| remote::remove(p, &name)).await
    }

    async fn set_remote_url(&self, repo: &RepoId, name: &str, url: &str) -> Result<()> {
        let handle = self.get_repo(repo)?;
        let name = name.to_string();
        let url = url.to_string();
        run_write_blocking(handle, move |p| remote::set_url(p, &name, &url)).await
    }

    async fn list_commit_files(&self, repo: &RepoId, commit: &str) -> Result<Vec<FileStatus>> {
        let handle = self.get_repo(repo)?;
        let commit = commit.to_string();
        run_blocking(move || commit_files::list(&handle.path, &commit)).await
    }

    async fn blame(&self, repo: &RepoId, path: &str) -> Result<Vec<BlameLine>> {
        let handle = self.get_repo(repo)?;
        let path = path.to_string();
        run_blocking(move || blame::run(&handle.path, &path)).await
    }

    async fn list_reflog(
        &self,
        repo: &RepoId,
        ref_name: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<ReflogEntry>> {
        let handle = self.get_repo(repo)?;
        let ref_name = ref_name.map(str::to_owned);
        run_blocking(move || reflog::list(&handle.path, ref_name.as_deref(), limit)).await
    }

    // ---- Clone / Init ----

    async fn clone_repo(&self, url: &str, dest: &Path) -> Result<RepoConfig> {
        let url = url.to_string();
        let dest_clone = dest.to_path_buf();
        run_blocking(move || clone::clone_repo(&url, &dest_clone)).await?;
        self.open_repo(dest).await
    }

    async fn init_repo(&self, path: &Path) -> Result<RepoConfig> {
        let path_init = path.to_path_buf();
        run_blocking(move || clone::init_repo(&path_init)).await?;
        self.open_repo(path).await
    }

    // ---- Interactive Rebase ----

    async fn interactive_rebase_plan(&self, repo: &RepoId, onto: &str) -> Result<Vec<RebaseTodo>> {
        let handle = self.get_repo(repo)?;
        let onto = onto.to_string();
        run_blocking(move || rebase_interactive::plan(&handle.path, &onto)).await
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
            rebase_interactive::execute(p, &onto, &todos)
        })
        .await
    }

    async fn get_conflict_content(&self, repo: &RepoId, path: &str) -> Result<ConflictContent> {
        let handle = self.get_repo(repo)?;
        let path = path.to_string();
        run_blocking(move || conflict_content::get_content(&handle.path, &path)).await
    }
}
