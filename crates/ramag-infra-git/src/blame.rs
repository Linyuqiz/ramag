//! Git blame 解析（subprocess git blame --porcelain）
//!
//! Porcelain 格式由若干 group 组成，每 group 一行：
//! ```text
//! <sha> <orig-line> <final-line>[ <count>]
//! author <name>
//! author-mail <<email>>
//! author-time <unix-ts>
//! author-tz <tz>
//! committer <name>
//! ...
//! summary <subject>
//! ...
//! filename <name>
//! \t<line content>
//! ```
//!
//! 已出现过的 sha 在后续 group 里只保留 sha 头 + `\t<content>`（其余 metadata 省略）。
//! 解析时维护一个 sha → metadata 缓存即可。

use std::collections::HashMap;
use std::path::Path;

use ramag_domain::entities::{BlameLine, CommitId};
use ramag_domain::error::Result;

use crate::git_cmd::run_git_text;

/// 取指定文件的 blame
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
        // 头行：<sha> <orig-line> <final-line> [<count>]
        let mut parts = header.split_whitespace();
        let Some(sha) = parts.next() else {
            continue;
        };
        let _orig = parts.next();
        let final_line: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);

        // 已知 sha → 直接取 cached meta；新 sha → 下面的 metadata 行用来填充
        let mut meta: CommitMeta = metas.get(sha).cloned().unwrap_or_default();
        let mut content = String::new();
        while i < lines.len() {
            let l = lines[i];
            i += 1;
            if let Some(c) = l.strip_prefix('\t') {
                // \t 开头是该行实际内容，结束本 group
                content = c.to_string();
                break;
            }
            // 解析 key value 行（仅记我们关心的 3 个字段）
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
        // 第一行有完整 metadata；第二行同一 sha 仅 sha 头 + content
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
        // 第二行复用 cache
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
