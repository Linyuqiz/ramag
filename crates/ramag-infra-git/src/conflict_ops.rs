//! 冲突解决：采纳 ours / theirs（subprocess git checkout + add）
//!
//! 两步走：先 checkout 对应版本覆盖工作区，再 git add 标记已解决。

use std::path::Path;

use ramag_domain::error::Result;

use crate::git_cmd::run_git_bytes;

/// 采纳「我们」（HEAD 侧）的版本
pub fn use_ours(repo_path: &Path, paths: &[String]) -> Result<()> {
    apply_side(repo_path, "--ours", paths)
}

/// 采纳「他们」（对方分支）的版本
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
