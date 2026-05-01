//! 工作区写操作（subprocess git）：stage / unstage / discard / branch ops
//!
//! 抽自 lib.rs（让其不超 600 行）。每个函数都是简单的 git args 构造 + 调 run_git_bytes。

use std::path::Path;

use ramag_domain::error::Result;

use crate::git_cmd::run_git_bytes;

/// `git add -- <paths>`
pub fn stage(repo_path: &Path, paths: &[String]) -> Result<()> {
    let mut args: Vec<&str> = vec!["add", "--"];
    for p in paths {
        args.push(p);
    }
    run_git_bytes(repo_path, &args).map(|_| ())
}

/// 列出仓库内所有 git 跟踪 + 未跟踪但未 ignore 的相对路径
///
/// `git ls-files --cached --others --exclude-standard -z`：
/// - `--cached`：包含所有 tracked 文件
/// - `--others`：包含未跟踪但不在 .gitignore 排除范围内的文件
/// - `--exclude-standard`：尊重 .gitignore / .git/info/exclude / 全局 excludes
/// - `-z`：用 NUL 分隔路径（避免文件名含空格 / 换行 / 中文等特殊字符引起解析错误）
pub fn list_files(repo_path: &Path) -> Result<Vec<String>> {
    let bytes = run_git_bytes(
        repo_path,
        &[
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    )?;
    // 按 NUL 切分；过滤空字符串（末尾通常多一个 NUL）
    let paths: Vec<String> = bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    Ok(paths)
}

/// `git reset HEAD -- <paths>`（不改工作区）
pub fn unstage(repo_path: &Path, paths: &[String]) -> Result<()> {
    let mut args: Vec<&str> = vec!["reset", "HEAD", "--"];
    for p in paths {
        args.push(p);
    }
    run_git_bytes(repo_path, &args).map(|_| ())
}

/// `git checkout -- <paths>`：把工作区还原到暂存区版本（已跟踪文件用）
pub fn discard(repo_path: &Path, paths: &[String]) -> Result<()> {
    let mut args: Vec<&str> = vec!["checkout", "--"];
    for p in paths {
        args.push(p);
    }
    run_git_bytes(repo_path, &args).map(|_| ())
}

/// `git checkout <target>`：切到分支 / commit / tag
pub fn checkout(repo_path: &Path, target: &str) -> Result<()> {
    run_git_bytes(repo_path, &["checkout", target]).map(|_| ())
}

/// `git branch <name> [<base>]`
pub fn create_branch(repo_path: &Path, name: &str, base: Option<&str>) -> Result<()> {
    let mut args: Vec<&str> = vec!["branch", name];
    if let Some(b) = base {
        args.push(b);
    }
    run_git_bytes(repo_path, &args).map(|_| ())
}

/// `git branch -d/-D <name>`
pub fn delete_branch(repo_path: &Path, name: &str, force: bool) -> Result<()> {
    let flag = if force { "-D" } else { "-d" };
    run_git_bytes(repo_path, &["branch", flag, name]).map(|_| ())
}
