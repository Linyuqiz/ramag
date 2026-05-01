//! 行级 / 分块 stage：用 git apply --cached 把 unified diff patch 写入 index
//!
//! `--recount` 让 git 自动重算 hunk header 的 line counts；
//! `--unidiff-zero` 兼容 zero-context patch（避免严格 unified 校验失败）。

use std::path::Path;

use ramag_domain::error::Result;

use crate::git_cmd::run_git_stdin;

/// 把 patch 写入暂存区
pub fn stage(repo_path: &Path, patch: &str) -> Result<()> {
    run_git_stdin(
        repo_path,
        &["apply", "--cached", "--recount", "--unidiff-zero", "-"],
        patch,
    )
    .map(|_| ())
}

/// 把 patch 从暂存区撤回（reverse 模式；不影响工作区）
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

/// 把 patch 反向应用到工作区（hunk 级回滚到 HEAD）
///
/// 用 `git apply --reverse` 不带 `--cached` 直接改工作区文件。
/// 失败常见原因：工作区改动与 patch 上下文不匹配（用户在 diff 后又改了文件）
pub fn discard(repo_path: &Path, patch: &str) -> Result<()> {
    run_git_stdin(
        repo_path,
        &["apply", "--reverse", "--recount", "--unidiff-zero", "-"],
        patch,
    )
    .map(|_| ())
}
