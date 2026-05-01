//! Clone / Init 操作（subprocess git）

use std::path::Path;

use ramag_domain::error::{DomainError, Result};

use crate::git_cmd::run_git_bytes;

/// Clone 远程仓库到 `dest` 目录（由 git 自动创建）
pub fn clone_repo(url: &str, dest: &Path) -> Result<()> {
    let dest_str = dest
        .to_str()
        .ok_or_else(|| DomainError::InvalidConfig("目标路径含非 UTF-8 字符".into()))?;
    run_git_bytes(
        dest.parent().unwrap_or(Path::new(".")),
        &["clone", url, dest_str],
    )
    .map(|_| ())
}

/// 在 `path` 目录初始化新 git 仓库（`git init`）
pub fn init_repo(path: &Path) -> Result<()> {
    let path_str = path
        .to_str()
        .ok_or_else(|| DomainError::InvalidConfig("目标路径含非 UTF-8 字符".into()))?;
    // git init 在目标目录内运行（不存在则自动创建）
    run_git_bytes(path.parent().unwrap_or(Path::new(".")), &["init", path_str]).map(|_| ())
}
