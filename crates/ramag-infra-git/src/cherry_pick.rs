//! Cherry-pick 操作（subprocess git cherry-pick）

use std::path::Path;

use ramag_domain::error::Result;

use crate::git_cmd::run_git_bytes;

pub fn start(repo_path: &Path, commit: &str) -> Result<()> {
    run_git_bytes(repo_path, &["cherry-pick", commit]).map(|_| ())
}

pub fn cont(repo_path: &Path) -> Result<()> {
    run_git_bytes(repo_path, &["cherry-pick", "--continue", "--no-edit"]).map(|_| ())
}

pub fn abort(repo_path: &Path) -> Result<()> {
    run_git_bytes(repo_path, &["cherry-pick", "--abort"]).map(|_| ())
}
