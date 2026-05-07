//! 行级 / 分块 stage：`git apply --cached`。`--recount` 重算行数、`--unidiff-zero` 容忍零上下文

use std::path::Path;

use ramag_domain::error::Result;

use crate::git_cmd::run_git_stdin;

pub fn stage(repo_path: &Path, patch: &str) -> Result<()> {
    run_git_stdin(
        repo_path,
        &["apply", "--cached", "--recount", "--unidiff-zero", "-"],
        patch,
    )
    .map(|_| ())
}

/// reverse 模式撤回，不影响工作区
pub fn unstage(repo_path: &Path, patch: &str) -> Result<()> {
    run_git_stdin(
        repo_path,
        &[
            "apply",
            "--cached",
            "--reverse",
            "--recount",
            "--unidiff-zero",
            "-",
        ],
        patch,
    )
    .map(|_| ())
}

/// hunk 级回滚到 HEAD（不走暂存区）。失败常见于工作区改动与 patch 上下文不匹配
pub fn discard(repo_path: &Path, patch: &str) -> Result<()> {
    run_git_stdin(
        repo_path,
        &["apply", "--reverse", "--recount", "--unidiff-zero", "-"],
        patch,
    )
    .map(|_| ())
}
