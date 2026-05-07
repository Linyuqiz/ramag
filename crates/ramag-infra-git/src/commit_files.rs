//! commit 涉及的文件列表，走 `git diff-tree --name-status`。`staged` 承载类型，`unstaged` 永远 None

use std::path::Path;

use ramag_domain::entities::{FileChangeKind, FileStatus};
use ramag_domain::error::Result;

use crate::git_cmd::run_git_text;

pub fn list(repo_path: &Path, commit: &str) -> Result<Vec<FileStatus>> {
    let raw = run_git_text(
        repo_path,
        &["diff-tree", "--no-commit-id", "--name-status", "-r", commit],
    )?;
    Ok(parse_diff_tree(&raw))
}

fn parse_diff_tree(text: &str) -> Vec<FileStatus> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let code_raw = match parts.next() {
            Some(c) => c,
            None => continue,
        };
        // R/C 后跟相似度数字，仅取首字母
        let code = code_raw.chars().next().unwrap_or(' ');
        let kind = match code {
            'M' => FileChangeKind::Modified,
            'A' => FileChangeKind::Added,
            'D' => FileChangeKind::Deleted,
            'R' => FileChangeKind::Renamed,
            'C' => FileChangeKind::Copied,
            'T' => FileChangeKind::TypeChanged,
            _ => continue,
        };
        let p1 = match parts.next() {
            Some(p) => p,
            None => continue,
        };
        let p2 = parts.next();
        let (path, old_path) = match (kind, p2) {
            (FileChangeKind::Renamed | FileChangeKind::Copied, Some(new_path)) => {
                (new_path.to_string(), Some(p1.to_string()))
            }
            _ => (p1.to_string(), None),
        };
        out.push(FileStatus {
            path,
            old_path,
            staged: Some(kind),
            unstaged: None,
        });
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_modify() {
        let text = "M\tsrc/lib.rs\nA\tsrc/new.rs\nD\tsrc/old.rs\n";
        let files = parse_diff_tree(text);
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].path, "src/lib.rs");
        assert_eq!(files[0].staged, Some(FileChangeKind::Modified));
        assert_eq!(files[1].staged, Some(FileChangeKind::Added));
        assert_eq!(files[2].staged, Some(FileChangeKind::Deleted));
    }

    #[test]
    fn parses_rename_with_old_path() {
        let text = "R100\told.rs\tnew.rs\n";
        let files = parse_diff_tree(text);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "new.rs");
        assert_eq!(files[0].old_path.as_deref(), Some("old.rs"));
        assert_eq!(files[0].staged, Some(FileChangeKind::Renamed));
    }

    #[test]
    fn skips_empty_lines() {
        assert!(parse_diff_tree("\n\n").is_empty());
    }
}
