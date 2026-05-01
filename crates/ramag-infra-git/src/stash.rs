//! Stash 列表查询
//!
//! 用 `git stash list --pretty=format:...` 拿结构化字段。`stash@{N}` 索引由 git 维护，
//! UI 操作（apply / drop）按 idx 反向引用回 git。

use std::path::Path;

use ramag_domain::entities::{CommitId, Stash, StashId};
use ramag_domain::error::Result;

use crate::git_cmd::{run_git_bytes, run_git_text};

/// 把当前未提交改动存进 stash
pub fn save(repo_path: &Path, message: Option<&str>, include_untracked: bool) -> Result<()> {
    let mut args: Vec<&str> = vec!["stash", "push"];
    if include_untracked {
        args.push("-u");
    }
    if let Some(m) = message {
        args.push("-m");
        args.push(m);
    }
    run_git_bytes(repo_path, &args).map(|_| ())
}

/// 应用某个 stash（pop=true 应用后删除）
pub fn apply(repo_path: &Path, idx: usize, pop: bool) -> Result<()> {
    let cmd = if pop { "pop" } else { "apply" };
    let r = format!("stash@{{{idx}}}");
    run_git_bytes(repo_path, &["stash", cmd, &r]).map(|_| ())
}

/// 仅删除某个 stash
pub fn drop(repo_path: &Path, idx: usize) -> Result<()> {
    let r = format!("stash@{{{idx}}}");
    run_git_bytes(repo_path, &["stash", "drop", &r]).map(|_| ())
}

pub fn list(repo_path: &Path) -> Result<Vec<Stash>> {
    // 用 `|` 分隔字段，避免与 stash message 内可能的 \x1f 等冲突
    // %gd: stash@{N} reflog selector
    // %H : stash 自身的 commit hash
    // %ct: committer timestamp
    // %s : subject (stash message)
    let out = run_git_text(
        repo_path,
        &["stash", "list", "--pretty=format:%gd|%H|%ct|%s"],
    )?;
    let mut result = Vec::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.splitn(4, '|').collect();
        if parts.len() != 4 {
            continue;
        }
        let idx_str = parts[0].trim_start_matches("stash@{").trim_end_matches('}');
        let Ok(idx) = idx_str.parse::<usize>() else {
            continue;
        };
        let commit = CommitId(parts[1].to_string());
        let ts = parts[2].parse::<i64>().unwrap_or(0);
        let timestamp = chrono::DateTime::from_timestamp(ts, 0).unwrap_or_default();
        let message = parts[3].to_string();
        result.push(Stash {
            id: StashId(idx),
            message,
            commit,
            timestamp,
        });
    }
    Ok(result)
}
