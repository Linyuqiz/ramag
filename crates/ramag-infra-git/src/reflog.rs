//! Reflog 解析（subprocess git reflog show --format=...）
//!
//! 用 `\x1f` 分隔字段：
//! - %H: commit hash
//! - %gd: ref selector（HEAD@{0}）
//! - %gs: reflog message（"checkout: moving from main to feature"）
//! - %cI: ISO 8601 commit date
//!
//! action 从 subject 里第一段（冒号前）抽：「commit: foo」→ action=commit, subject=foo

use std::path::Path;

use ramag_domain::entities::{CommitId, ReflogEntry};
use ramag_domain::error::Result;

use crate::git_cmd::run_git_text;

const REFLOG_FORMAT: &str = "%H%x1f%gd%x1f%gs%x1f%cI";

/// 列出指定 ref 的 reflog 条目（默认 HEAD）
pub fn list(
    repo_path: &Path,
    ref_name: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<ReflogEntry>> {
    let mut args: Vec<String> = vec![
        "reflog".into(),
        "show".into(),
        format!("--format={REFLOG_FORMAT}"),
    ];
    if let Some(n) = limit {
        args.push(format!("--max-count={n}"));
    }
    args.push(ref_name.unwrap_or("HEAD").to_string());
    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
    let raw = run_git_text(repo_path, &args_ref)?;
    Ok(parse_reflog(&raw))
}

fn parse_reflog(text: &str) -> Vec<ReflogEntry> {
    text.lines()
        .filter_map(|line| {
            if line.is_empty() {
                return None;
            }
            let mut parts = line.splitn(4, '\x1f');
            let hash = parts.next()?.trim();
            let selector = parts.next()?.to_string();
            let raw_subject = parts.next()?.to_string();
            let date_iso = parts.next()?;
            let timestamp = chrono::DateTime::parse_from_rfc3339(date_iso)
                .map(|t| t.with_timezone(&chrono::Utc))
                .unwrap_or_default();
            // subject 形如 "commit: foo bar" / "checkout: moving from a to b"
            let (action, subject) = match raw_subject.split_once(':') {
                Some((a, s)) => (a.trim().to_string(), s.trim().to_string()),
                None => (String::new(), raw_subject),
            };
            Some(ReflogEntry {
                commit: CommitId(hash.to_string()),
                selector,
                action,
                subject,
                timestamp,
            })
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_reflog() {
        let text = "abc123\u{1f}HEAD@{0}\u{1f}commit: fix bug\u{1f}2026-01-01T12:00:00+00:00\n\
                    def456\u{1f}HEAD@{1}\u{1f}checkout: moving from main to feature\u{1f}2026-01-01T11:00:00+00:00\n";
        let entries = parse_reflog(text);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].commit.0, "abc123");
        assert_eq!(entries[0].selector, "HEAD@{0}");
        assert_eq!(entries[0].action, "commit");
        assert_eq!(entries[0].subject, "fix bug");
        assert_eq!(entries[1].action, "checkout");
        assert_eq!(entries[1].subject, "moving from main to feature");
    }

    #[test]
    fn handles_subject_without_colon() {
        let text = "abc\u{1f}HEAD@{0}\u{1f}initial\u{1f}2026-01-01T00:00:00+00:00\n";
        let entries = parse_reflog(text);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].action.is_empty());
        assert_eq!(entries[0].subject, "initial");
    }

    #[test]
    fn empty_input() {
        assert_eq!(parse_reflog("").len(), 0);
    }
}
