//! 取冲突文件的三方内容（stage 1/2/3 = base/ours/theirs）
//!
//! 使用 `git show :N:<path>` 读各 stage 的版本：
//! - `:1:` = 共同祖先（merge base）
//! - `:2:` = HEAD（ours）
//! - `:3:` = MERGE_HEAD（theirs）

use std::path::Path;

use ramag_domain::entities::ConflictContent;
use ramag_domain::error::Result;

use crate::git_cmd::run_git_bytes;

/// 取三方冲突内容；若某个 stage 不存在（如 add/delete 冲突），对应侧返回空 Vec
pub fn get_content(repo_path: &Path, file_path: &str) -> Result<ConflictContent> {
    Ok(ConflictContent {
        path: file_path.to_string(),
        base: read_stage(repo_path, 1, file_path),
        ours: read_stage(repo_path, 2, file_path),
        theirs: read_stage(repo_path, 3, file_path),
    })
}

fn read_stage(repo_path: &Path, stage: u8, file_path: &str) -> Vec<String> {
    let spec = format!(":{stage}:{file_path}");
    match run_git_bytes(repo_path, &["show", &spec]) {
        Ok(bytes) => String::from_utf8_lossy(&bytes)
            .lines()
            .map(str::to_string)
            .collect(),
        Err(_) => Vec::new(), // stage 不存在（add/delete 冲突一侧为空）
    }
}

/// 快速检查文件是否处于冲突状态（有 stage 2 或 stage 3 即认为冲突）
pub fn is_conflicted(repo_path: &Path, file_path: &str) -> bool {
    let spec = format!(":2:{file_path}");
    run_git_bytes(repo_path, &["show", &spec]).is_ok()
}
