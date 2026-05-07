//! `git rebase`。冲突时进入 RepoOperation::Rebase，UI 推进 continue / skip / abort

use std::path::Path;

use ramag_domain::error::Result;

use crate::git_cmd::run_git_bytes;

pub fn start(repo_path: &Path, onto: &str) -> Result<()> {
    run_git_bytes(repo_path, &["rebase", onto]).map(|_| ())
}

pub fn cont(repo_path: &Path) -> Result<()> {
    run_git_bytes(repo_path, &["rebase", "--continue"]).map(|_| ())
}

pub fn skip(repo_path: &Path) -> Result<()> {
    run_git_bytes(repo_path, &["rebase", "--skip"]).map(|_| ())
}

pub fn abort(repo_path: &Path) -> Result<()> {
    run_git_bytes(repo_path, &["rebase", "--abort"]).map(|_| ())
}
