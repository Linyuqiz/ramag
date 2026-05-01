//! Merge 操作（subprocess git merge）
//!
//! 与 cherry_pick / rebase 并列；冲突时仓库进入 RepoOperation::Merge 状态，
//! UI 通过 continue / abort 推进。

use std::path::Path;

use ramag_domain::error::Result;

use crate::git_cmd::run_git_bytes;

/// 合并指定分支到当前 HEAD
///
/// `no_ff=true` 强制建 merge commit；`ff_only=true` 要求必须 ff，否则失败
pub fn start(
    repo_path: &Path,
    branch: &str,
    no_ff: bool,
    ff_only: bool,
    message: Option<&str>,
) -> Result<()> {
    let mut args: Vec<&str> = vec!["merge"];
    if no_ff {
        args.push("--no-ff");
    }
    if ff_only {
        args.push("--ff-only");
    }
    if let Some(m) = message {
        args.push("-m");
        args.push(m);
    }
    args.push(branch);
    run_git_bytes(repo_path, &args).map(|_| ())
}

pub fn cont(repo_path: &Path) -> Result<()> {
    run_git_bytes(repo_path, &["merge", "--continue", "--no-edit"]).map(|_| ())
}

pub fn abort(repo_path: &Path) -> Result<()> {
    run_git_bytes(repo_path, &["merge", "--abort"]).map(|_| ())
}
