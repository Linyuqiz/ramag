//! `git blame --porcelain` 解析。同 sha 的后续 group 仅 sha 头 + `\t<content>`，metadata 缓存复用

use std::collections::HashMap;
use std::path::Path;

use ramag_domain::entities::{BlameLine, CommitId};
use ramag_domain::error::Result;

use crate::git_cmd::run_git_text;

pub fn run(repo_path: &Path, file: &str) -> Result<Vec<BlameLine>> {
    let raw = run_git_text(repo_path, &["blame", "--porcelain", "--", file])?;
    Ok(parse_porcelain(&raw))
}

#[derive(Debug, Default, Clone)]
struct CommitMeta {
    author: String,
    timestamp: i64,
    subject: String,
}

fn parse_porcelain(text: &str) -> Vec<BlameLine> {
    let mut metas: HashMap<String, CommitMeta> = HashMap::new();
    let mut out: Vec<BlameLine> = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let header = lines[i];
        i += 1;
        // 头行：<sha> <orig> <final> [<count>]
        let mut parts = header.split_whitespace();
        let Some(sha) = parts.next() else {
            continue;
        };
        let _orig = parts.next();
        let final_line: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);

        // 已知 sha 取 cached；新 sha 用下面的 metadata 行填充
        let mut meta: CommitMeta = metas.get(sha).cloned().unwrap_or_default();
        let mut content = String::new();
        while i < lines.len() {
            let l = lines[i];
            i += 1;
            if let Some(c) = l.strip_prefix('\t') {
                // \t 是该行实际内容，结束本 group
                content = c.to_string();
                break;
            }
            let mut kv = l.splitn(2, ' ');
            let key = kv.next().unwrap_or("");
            let val = kv.next().unwrap_or("");
            match key {
                "author" => meta.author = val.to_string(),
                "author-time" => {
                    meta.timestamp = val.parse().unwrap_or(0);
                }
                "summary" => meta.subject = val.to_string(),
                _ => {}
            }
        }
        metas.insert(sha.to_string(), meta.clone());
        let timestamp = chrono::DateTime::from_timestamp(meta.timestamp, 0).unwrap_or_default();
        out.push(BlameLine {
            commit: CommitId(sha.to_string()),
            author: meta.author,
            timestamp,
            line_no: final_line,
            subject: meta.subject,
            content,
        });
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_lines_same_commit() {
        let text = "\
abc123 1 1 2
author Alice
author-mail <a@x.com>
author-time 1700000000
author-tz +0000
committer Alice
committer-mail <a@x.com>
committer-time 1700000000
committer-tz +0000
summary first commit
filename foo.rs
\tline one
abc123 2 2
\tline two
";
        let blame = parse_porcelain(text);
        assert_eq!(blame.len(), 2);
        assert_eq!(blame[0].commit.0, "abc123");
        assert_eq!(blame[0].author, "Alice");
        assert_eq!(blame[0].subject, "first commit");
        assert_eq!(blame[0].content, "line one");
        assert_eq!(blame[0].line_no, 1);
        assert_eq!(blame[1].commit.0, "abc123");
        assert_eq!(blame[1].author, "Alice");
        assert_eq!(blame[1].content, "line two");
        assert_eq!(blame[1].line_no, 2);
    }

    #[test]
    fn empty_input() {
        assert_eq!(parse_porcelain("").len(), 0);
    }
}
