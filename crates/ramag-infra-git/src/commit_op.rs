//! 创建 commit（subprocess git commit）
//!
//! amend / sign 通过参数控制，subject 用 `-m` 注入避免编辑器弹出。
//! 成功后立即 `rev-parse HEAD` 拿新 commit hash 返回给 UI 用。

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
