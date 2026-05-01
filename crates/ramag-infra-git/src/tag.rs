//! Tag 解析（subprocess git for-each-ref）
//!
//! 同一格式同时识别 lightweight 和 annotated：
//! - `objecttype=tag`：annotated，`objectname` 是 tag 对象，`*objectname` 是它指向的 commit
//! - `objecttype=commit`：lightweight，`objectname` 已经是 commit，`*objectname` 为空

use std::path::Path;

use ramag_domain::entities::{CommitId, Signature, Tag, TagKind};
use ramag_domain::error::Result;

use crate::git_cmd::{run_git_bytes, run_git_text};

/// 创建 tag
///
/// - `message=Some(_)` → annotated tag（含 -m）
/// - `sign=true` → GPG 签名（隐含 annotated；message=None 时也强制 annotated 才能签名）
pub fn create(
    repo_path: &Path,
    name: &str,
    target: Option<&str>,
    message: Option<&str>,
    sign: bool,
) -> Result<()> {
    let mut args: Vec<&str> = vec!["tag"];
    let placeholder_msg: String;
    if sign {
        // -s 隐含 annotated；message=None 时给个占位 subject 让 git 不弹编辑器
        args.push("-s");
        let msg_to_use = match message {
            Some(m) => m,
            None => {
                placeholder_msg = format!("Tag {name}");
                placeholder_msg.as_str()
            }
        };
        args.push("-m");
        args.push(msg_to_use);
    } else if let Some(m) = message {
        args.push("-a");
        args.push("-m");
        args.push(m);
    }
    args.push(name);
    if let Some(t) = target {
        args.push(t);
    }
    run_git_bytes(repo_path, &args).map(|_| ())
}

/// 删除本地 tag
pub fn delete(repo_path: &Path, name: &str) -> Result<()> {
    run_git_bytes(repo_path, &["tag", "-d", name]).map(|_| ())
}

/// 推送 tag 到指定远程
pub fn push(repo_path: &Path, remote: &str, name: &str) -> Result<()> {
    let refname = format!("refs/tags/{name}");
    run_git_bytes(repo_path, &["push", remote, &refname]).map(|_| ())
}

/// 列出所有 tag（按名称字母序）
pub fn list(repo_path: &Path) -> Result<Vec<Tag>> {
    // 用 NUL 分隔字段（避免 message 里包含换行 / tab）；用 LF 分隔行
    let fmt = "%(refname:short)%00%(objecttype)%00%(objectname)%00\
               %(*objectname)%00%(taggername)%00%(taggeremail)%00\
               %(taggerdate:iso-strict)%00%(*subject)%00%(subject)";
    let format_arg = format!("--format={fmt}");
    let raw = run_git_text(repo_path, &["for-each-ref", &format_arg, "refs/tags/"])?;
    Ok(parse_tags(&raw))
}

fn parse_tags(text: &str) -> Vec<Tag> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\u{0}').collect();
        if parts.len() < 9 {
            continue;
        }
        let name = parts[0].to_string();
        let object_type = parts[1];
        let objectname = parts[2];
        let starobjectname = parts[3];
        let tagger_name = parts[4];
        let tagger_email = parts[5].trim_matches(|c| c == '<' || c == '>');
        let tagger_date = parts[6];
        let star_subject = parts[7];
        let plain_subject = parts[8];

        let (kind, commit_hash, message, tagger) = if object_type == "tag" {
            // annotated tag：subject 是 tag 自己的 message；*subject 是指向 commit 的 subject
            let commit = if starobjectname.is_empty() {
                objectname.to_string()
            } else {
                starobjectname.to_string()
            };
            // 优先 tag 自己的 message；空时 fallback 到 commit subject（少见）
            let msg = if !plain_subject.is_empty() {
                Some(plain_subject.to_string())
            } else if !star_subject.is_empty() {
                Some(star_subject.to_string())
            } else {
                None
            };
            let sig = parse_signature(tagger_name, tagger_email, tagger_date);
            (TagKind::Annotated, commit, msg, sig)
        } else {
            // lightweight：objectname 直接是 commit；%(subject) 就是 commit 的 subject
            let msg = if plain_subject.is_empty() {
                None
            } else {
                Some(plain_subject.to_string())
            };
            (TagKind::Lightweight, objectname.to_string(), msg, None)
        };

        out.push(Tag {
            name,
            kind,
            commit: CommitId(commit_hash),
            message,
            tagger,
        });
    }
    out
}

fn parse_signature(name: &str, email: &str, date_iso: &str) -> Option<Signature> {
    if name.is_empty() && email.is_empty() && date_iso.is_empty() {
        return None;
    }
    let timestamp = chrono::DateTime::parse_from_rfc3339(date_iso)
        .map(|t| t.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());
    Some(Signature {
        name: name.to_string(),
        email: email.to_string(),
        timestamp,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parses_lightweight_with_commit_subject() {
        // lightweight tag v1.0 → commit abc，commit subject = "fix bug"
        // git for-each-ref 的 %(subject) 对 lightweight 直接返回 commit subject
        let text = "v1.0\u{0}commit\u{0}abc\u{0}\u{0}\u{0}\u{0}\u{0}\u{0}fix bug\n";
        let tags = parse_tags(text);
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].kind, TagKind::Lightweight);
        assert_eq!(tags[0].message.as_deref(), Some("fix bug"));
        assert!(tags[0].tagger.is_none());
    }

    #[test]
    fn parses_annotated_with_tag_message() {
        // annotated v2.0 → tag def → commit abc；tag message = "release", commit subject = "raw"
        let text = "v2.0\u{0}tag\u{0}def\u{0}abc\u{0}Alice\u{0}<a@e.com>\u{0}2026-01-01T00:00:00+00:00\u{0}raw\u{0}release\n";
        let tags = parse_tags(text);
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].kind, TagKind::Annotated);
        assert_eq!(tags[0].commit.0, "abc"); // *objectname
        // 优先 tag 自己的 message
        assert_eq!(tags[0].message.as_deref(), Some("release"));
        let sig = tags[0].tagger.as_ref().unwrap();
        assert_eq!(sig.name, "Alice");
        assert_eq!(sig.email, "a@e.com");
    }

    #[test]
    fn skips_empty_lines() {
        let text = "\n\n";
        assert!(parse_tags(text).is_empty());
    }
}
