//! 工作区写操作：stage / unstage / discard / checkout / branch / list_files

use std::path::Path;

use ramag_domain::error::Result;

use crate::git_cmd::run_git_bytes;

pub fn stage(repo_path: &Path, paths: &[String]) -> Result<()> {
    let mut args: Vec<&str> = vec!["add", "--"];
    for p in paths {
        args.push(p);
    }
    run_git_bytes(repo_path, &args).map(|_| ())
}

/// `git ls-files --cached --others --exclude-standard -z`
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
    // NUL 切分；末尾常多一个 NUL，过滤空串
    let paths: Vec<String> = bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    Ok(paths)
}

/// 不改工作区
pub fn unstage(repo_path: &Path, paths: &[String]) -> Result<()> {
    let mut args: Vec<&str> = vec!["reset", "HEAD", "--"];
    for p in paths {
        args.push(p);
    }
    run_git_bytes(repo_path, &args).map(|_| ())
}

/// 工作区还原到暂存区版本（仅 tracked 文件）
pub fn discard(repo_path: &Path, paths: &[String]) -> Result<()> {
    let mut args: Vec<&str> = vec!["checkout", "--"];
    for p in paths {
        args.push(p);
    }
    run_git_bytes(repo_path, &args).map(|_| ())
}

pub fn checkout(repo_path: &Path, target: &str) -> Result<()> {
    run_git_bytes(repo_path, &["checkout", target]).map(|_| ())
}

pub fn create_branch(repo_path: &Path, name: &str, base: Option<&str>) -> Result<()> {
    let mut args: Vec<&str> = vec!["branch", name];
    if let Some(b) = base {
        args.push(b);
    }
    run_git_bytes(repo_path, &args).map(|_| ())
}

pub fn delete_branch(repo_path: &Path, name: &str, force: bool) -> Result<()> {
    let flag = if force { "-D" } else { "-d" };
    run_git_bytes(repo_path, &["branch", flag, name]).map(|_| ())
}
