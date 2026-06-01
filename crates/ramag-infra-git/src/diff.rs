//! `git diff` unified 输出 → FileDiff。binary 仅识别占位行不渲染；mode 字段留空

use std::path::Path;

use ramag_domain::entities::{
    CommitId, DiffKind, DiffLine, DiffLineKind, FileChangeKind, FileDiff, Hunk,
};
use ramag_domain::error::Result;

use crate::git_cmd::run_git_bytes;

pub fn run_diff(repo_path: &Path, path: &str, kind: &DiffKind) -> Result<FileDiff> {
    run_diff_opts(repo_path, path, kind, false)
}

pub fn run_diff_opts(
    repo_path: &Path,
    path: &str,
    kind: &DiffKind,
    ignore_whitespace: bool,
) -> Result<FileDiff> {
    run_diff_full_opts(repo_path, path, kind, ignore_whitespace, 3)
}

/// `context_lines`：3=标准、0=仅变更、999999=全文件
pub fn run_diff_full_opts(
    repo_path: &Path,
    path: &str,
    kind: &DiffKind,
    ignore_whitespace: bool,
    context_lines: u32,
) -> Result<FileDiff> {
    let mut args_strings = build_diff_args(path, kind, context_lines);
    if ignore_whitespace {
        args_strings.insert(1, "-w".into());
    }
    let args: Vec<&str> = args_strings.iter().map(String::as_str).collect();
    let bytes = run_git_bytes(repo_path, &args)?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(parse_unified_diff(&text, path))
}

fn build_diff_args(path: &str, kind: &DiffKind, context_lines: u32) -> Vec<String> {
    // CommitVsParent 走 diff-tree --root：根 commit（无父）与空树对比，
    // 否则 `git diff <c>^ <c>` 对根 commit 因 `<c>^` 不存在而报错（点第一个 commit 看 diff 会失败）
    if let DiffKind::CommitVsParent(CommitId(c)) = kind {
        return vec![
            "diff-tree".into(),
            "--no-color".into(),
            format!("-U{context_lines}"),
            "--find-renames".into(),
            "--root".into(),
            "-p".into(),
            "--no-commit-id".into(),
            c.clone(),
            "--".into(),
            path.into(),
        ];
    }
    let mut args: Vec<String> = vec![
        "diff".into(),
        "--no-color".into(),
        format!("-U{context_lines}"),
        "--find-renames".into(),
    ];
    match kind {
        DiffKind::WorkingTreeVsIndex => {}
        DiffKind::IndexVsHead => args.push("--cached".into()),
        DiffKind::WorkingTreeVsHead => args.push("HEAD".into()),
        DiffKind::CommitVsParent(_) => unreachable!("已在函数开头用 diff-tree 处理"),
        DiffKind::Range {
            from: CommitId(f),
            to: CommitId(t),
        } => {
            args.push(f.clone());
            args.push(t.clone());
        }
    }
    args.push("--".into());
    args.push(path.into());
    args
}

fn parse_unified_diff(text: &str, path: &str) -> FileDiff {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current: Option<Hunk> = None;
    let mut old_no: u32 = 0;
    let mut new_no: u32 = 0;
    let mut binary = false;
    let mut change_kind = FileChangeKind::Modified;
    let mut old_path: Option<String> = None;

    for line in text.lines() {
        if line.starts_with("Binary files") {
            binary = true;
            continue;
        }
        if line.starts_with("new file") {
            change_kind = FileChangeKind::Added;
            continue;
        }
        if line.starts_with("deleted file") {
            change_kind = FileChangeKind::Deleted;
            continue;
        }
        if line.starts_with("rename from ") {
            change_kind = FileChangeKind::Renamed;
            old_path = Some(line.trim_start_matches("rename from ").to_string());
            continue;
        }
        if let Some(stripped) = line.strip_prefix("@@") {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            // `@@ -os[,ol] +ns[,nl] @@ heading`
            let close = stripped.find("@@");
            let core = match close {
                Some(i) => stripped[..i].trim(),
                None => stripped.trim(),
            };
            let parts: Vec<&str> = core.split_whitespace().collect();
            let (os, ol) = parts
                .first()
                .and_then(|s| s.strip_prefix('-'))
                .map(parse_range)
                .unwrap_or((0, 0));
            let (ns, nl) = parts
                .get(1)
                .and_then(|s| s.strip_prefix('+'))
                .map(parse_range)
                .unwrap_or((0, 0));
            let heading = match close {
                Some(i) if stripped.len() > i + 2 => {
                    let h = stripped[i + 2..].trim();
                    if h.is_empty() {
                        None
                    } else {
                        Some(h.to_string())
                    }
                }
                _ => None,
            };
            old_no = os;
            new_no = ns;
            current = Some(Hunk {
                old_start: os,
                old_lines: ol,
                new_start: ns,
                new_lines: nl,
                heading,
                lines: Vec::new(),
            });
            continue;
        }
        if line.starts_with("diff --git") || line.starts_with("index ") {
            continue;
        }
        if line.starts_with("--- ") || line.starts_with("+++ ") {
            continue;
        }
        let Some(h) = current.as_mut() else { continue };
        match line.chars().next() {
            Some(' ') => {
                h.lines.push(DiffLine {
                    kind: DiffLineKind::Context,
                    old_lineno: Some(old_no),
                    new_lineno: Some(new_no),
                    text: line[1..].to_string(),
                });
                old_no += 1;
                new_no += 1;
            }
            Some('-') => {
                h.lines.push(DiffLine {
                    kind: DiffLineKind::Delete,
                    old_lineno: Some(old_no),
                    new_lineno: None,
                    text: line[1..].to_string(),
                });
                old_no += 1;
            }
            Some('+') => {
                h.lines.push(DiffLine {
                    kind: DiffLineKind::Add,
                    old_lineno: None,
                    new_lineno: Some(new_no),
                    text: line[1..].to_string(),
                });
                new_no += 1;
            }
            // `\ No newline at end of file` 等忽略
            _ => {}
        }
    }
    if let Some(h) = current {
        hunks.push(h);
    }

    FileDiff {
        path: path.to_string(),
        old_path,
        change_kind,
        binary,
        old_mode: None,
        new_mode: None,
        hunks,
    }
}

fn parse_range(s: &str) -> (u32, u32) {
    let (start, count) = match s.split_once(',') {
        Some((a, b)) => (a, b),
        None => (s, "1"),
    };
    (start.parse().unwrap_or(0), count.parse().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_modify_diff() {
        let text = "\
diff --git a/file.txt b/file.txt
index abc..def 100644
--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,4 @@
 line1
-old
+new
+added
 line3
";
        let d = parse_unified_diff(text, "file.txt");
        assert_eq!(d.path, "file.txt");
        assert_eq!(d.hunks.len(), 1);
        let h = &d.hunks[0];
        assert_eq!(h.old_start, 1);
        assert_eq!(h.old_lines, 3);
        assert_eq!(h.new_start, 1);
        assert_eq!(h.new_lines, 4);
        assert_eq!(h.lines.len(), 5);
        let kinds: Vec<DiffLineKind> = h.lines.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            vec![
                DiffLineKind::Context,
                DiffLineKind::Delete,
                DiffLineKind::Add,
                DiffLineKind::Add,
                DiffLineKind::Context,
            ]
        );
    }

    #[test]
    fn parses_new_file() {
        let text = "\
diff --git a/new.txt b/new.txt
new file mode 100644
index 000..abc
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,2 @@
+hello
+world
";
        let d = parse_unified_diff(text, "new.txt");
        assert_eq!(d.change_kind, FileChangeKind::Added);
        assert_eq!(d.hunks.len(), 1);
        assert_eq!(d.hunks[0].lines.len(), 2);
    }
}
