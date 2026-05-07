//! 冲突解决：`git checkout --ours/--theirs` + `git add` 一步标记已解决

use std::path::Path;

use ramag_domain::error::Result;

use crate::git_cmd::run_git_bytes;

/// HEAD 侧
pub fn use_ours(repo_path: &Path, paths: &[String]) -> Result<()> {
    apply_side(repo_path, "--ours", paths)
}

/// 对方分支侧
pub fn use_theirs(repo_path: &Path, paths: &[String]) -> Result<()> {
    apply_side(repo_path, "--theirs", paths)
}

fn apply_side(repo_path: &Path, side: &str, paths: &[String]) -> Result<()> {
    let mut args1: Vec<&str> = vec!["checkout", side, "--"];
    for p in paths {
        args1.push(p);
    }
    run_git_bytes(repo_path, &args1)?;
    let mut args2: Vec<&str> = vec!["add", "--"];
    for p in paths {
        args2.push(p);
    }
    run_git_bytes(repo_path, &args2).map(|_| ())
}
