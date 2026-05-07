//! Stash 列表 + 操作。`stash@{N}` 索引由 git 维护，UI 按 idx 反查

use std::path::Path;

use ramag_domain::entities::{CommitId, Stash, StashId};
use ramag_domain::error::Result;

use crate::git_cmd::{run_git_bytes, run_git_text};

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

/// pop=true 应用后删除
pub fn apply(repo_path: &Path, idx: usize, pop: bool) -> Result<()> {
    let cmd = if pop { "pop" } else { "apply" };
    let r = format!("stash@{{{idx}}}");
    run_git_bytes(repo_path, &["stash", cmd, &r]).map(|_| ())
}

pub fn drop(repo_path: &Path, idx: usize) -> Result<()> {
    let r = format!("stash@{{{idx}}}");
    run_git_bytes(repo_path, &["stash", "drop", &r]).map(|_| ())
}

pub fn list(repo_path: &Path) -> Result<Vec<Stash>> {
    // `|` 分字段：%gd selector / %H commit / %ct ts / %s message
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
