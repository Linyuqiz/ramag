//! 三方冲突内容：`git show :N:<path>`，1=base、2=ours、3=theirs

use std::path::Path;

use ramag_domain::entities::ConflictContent;
use ramag_domain::error::Result;

use crate::git_cmd::run_git_bytes;

/// stage 不存在（add/delete 冲突）时对应侧返回空 Vec
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
        Err(_) => Vec::new(),
    }
}

/// 有 stage 2 即视为冲突
pub fn is_conflicted(repo_path: &Path, file_path: &str) -> bool {
    let spec = format!(":2:{file_path}");
    run_git_bytes(repo_path, &["show", &spec]).is_ok()
}
