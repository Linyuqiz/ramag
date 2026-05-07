//! `git commit`。subject 走 `-m` 避免编辑器弹出；成功后 `rev-parse HEAD` 取新 hash

use std::path::Path;

use ramag_domain::entities::CommitId;
use ramag_domain::error::Result;

use crate::git_cmd::{run_git_bytes, run_git_text};

pub fn run(repo_path: &Path, message: &str, amend: bool, sign: bool) -> Result<CommitId> {
    let mut args: Vec<&str> = vec!["commit"];
    if amend {
        args.push("--amend");
    }
    if sign {
        args.push("-S");
    }
    args.push("-m");
    args.push(message);
    run_git_bytes(repo_path, &args)?;
    let id = run_git_text(repo_path, &["rev-parse", "HEAD"])?;
    Ok(CommitId(id.trim().to_string()))
}
