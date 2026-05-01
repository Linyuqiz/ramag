//! 工作区状态 + 分支查询
//!
//! - HEAD 信息（分支名 / commit 短 hash）：[`gix`] 直接读 ref
//! - 文件级变更（staged / unstaged / untracked / conflicted）：subprocess `git status --porcelain=v2 -z`
//! - 进行中操作（merge / rebase 等）：检查 .git 目录下的标记文件
//! - ahead / behind：subprocess `git rev-list --left-right --count`

use std::path::Path;

use ramag_domain::entities::{
    Branch, BranchKind, CommitId, FileChangeKind, FileStatus, RepoOperation, WorkingTreeStatus,
};
use ramag_domain::error::Result;

use crate::errors::{map_branch_error, map_status_error};
use crate::git_cmd::{run_git_bytes, run_git_text};

/// 工作区状态采集（HEAD + 变更文件 + ahead/behind + 进行中操作）
pub fn collect_status(repo: &gix::Repository, repo_path: &Path) -> Result<WorkingTreeStatus> {
    let head_branch = match repo.head_name() {
        Ok(Some(name)) => Some(short_branch_name(name.as_bstr())),
        Ok(None) => None, // detached HEAD
        Err(e) => return Err(map_status_error(e)),
    };

    let head_commit = repo
        .head_id()
        .ok()
        .map(|id| CommitId(id.to_string()).short().to_string());

    let operation = detect_operation(repo);
    let files = parse_porcelain_v2(repo_path).unwrap_or_default();
    let (ahead, behind) = count_ahead_behind(repo_path);

    Ok(WorkingTreeStatus {
        head_branch,
        head_commit,
        operation,
        files,
        ahead,
        behind,
    })
}

/// 列出分支，本地分支同时填充 upstream / ahead / behind
pub fn list_branches(repo: &gix::Repository, kind: BranchKind) -> Result<Vec<Branch>> {
    // HEAD 指向的分支名（symbolic ref），用来精确判定 is_head。
    // 不能仅用 commit_id 比较：多个分支可能指向同一个 commit（如刚 `git branch te` 从 main），
    // 这会导致 main / te 同时被标记 is_head=true。
    let head_branch_name = match repo.head_name() {
        Ok(Some(name)) => Some(short_branch_name(name.as_bstr())),
        _ => None,
    };
    let platform = repo.references().map_err(map_branch_error)?;

    let iter = match kind {
        BranchKind::Local => platform.local_branches(),
        BranchKind::Remote => platform.remote_branches(),
    }
    .map_err(map_branch_error)?;

    // 本地分支才有 upstream tracking（远程分支本身就是上游）
    let tracking = if matches!(kind, BranchKind::Local) {
        fetch_branch_tracking(repo)
    } else {
        std::collections::HashMap::new()
    };

    let mut branches = Vec::new();
    for r in iter {
        let r = match r {
            Ok(r) => r,
            Err(_) => continue,
        };
        let full = r.name().as_bstr();
        let short = short_branch_name(full);
        let commit_id = match r.target().try_id() {
            Some(id) => CommitId(id.to_string()),
            None => continue,
        };
        // 本地：精确按分支名匹配 HEAD；远程：分支永远不是 HEAD
        let is_head = matches!(kind, BranchKind::Local)
            && head_branch_name.as_deref() == Some(short.as_str());

        let (upstream, ahead, behind) = if let Some(t) = tracking.get(&short) {
            (Some(t.upstream.clone()), t.ahead, t.behind)
        } else {
            (None, None, None)
        };

        branches.push(Branch {
            name: short,
            kind,
            commit: commit_id,
            is_head,
            upstream,
            ahead,
            behind,
        });
    }
    Ok(branches)
}

/// upstream tracking 信息（per local branch）
struct TrackInfo {
    upstream: String,
    ahead: Option<usize>,
    behind: Option<usize>,
}

/// 用 `git for-each-ref` 批量取本地分支的 upstream + track 计数
fn fetch_branch_tracking(repo: &gix::Repository) -> std::collections::HashMap<String, TrackInfo> {
    let repo_path = repo.git_dir().parent().unwrap_or(repo.git_dir());
    let out = match run_git_text(
        repo_path,
        &[
            "for-each-ref",
            "--format=%(refname:short)\t%(upstream:short)\t%(upstream:track)",
            "refs/heads/",
        ],
    ) {
        Ok(s) => s,
        Err(_) => return std::collections::HashMap::new(),
    };

    let mut map = std::collections::HashMap::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() < 2 || parts[1].is_empty() {
            continue;
        }
        let branch = parts[0].to_string();
        let upstream = parts[1].to_string();
        let track_str = if parts.len() > 2 { parts[2] } else { "" };
        let (ahead, behind) = parse_track(track_str);
        map.insert(
            branch,
            TrackInfo {
                upstream,
                ahead,
                behind,
            },
        );
    }
    map
}

/// 解析 `%(upstream:track)` 输出，如 `[ahead 2, behind 1]`、`[ahead 3]`、`[behind 1]`
fn parse_track(s: &str) -> (Option<usize>, Option<usize>) {
    let s = s.trim().trim_matches('[').trim_matches(']');
    let mut ahead = None;
    let mut behind = None;
    for part in s.split(',') {
        let part = part.trim();
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.trim().parse().ok();
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.trim().parse().ok();
        }
    }
    (ahead, behind)
}

// =============================================================================
// 内部解析 helpers
// =============================================================================

/// 解析 `git status --porcelain=v2 -z` 输出
///
/// 每条记录由 NUL 分隔，第一字节是 entry type：
/// - `1` ordinary（modified/added/deleted 等普通变更）
/// - `2` rename / copy（后续紧跟一条 NUL 分隔的旧路径）
/// - `?` untracked
/// - `u` unmerged（冲突）
/// - `!` ignored（本工具不处理）
fn parse_porcelain_v2(repo_path: &Path) -> Result<Vec<FileStatus>> {
    let bytes = run_git_bytes(repo_path, &["status", "--porcelain=v2", "-z"])?;
    let mut out = Vec::new();
    let mut iter = bytes.split(|&b| b == 0).filter(|s| !s.is_empty());
    while let Some(record) = iter.next() {
        let first = record.first().copied().unwrap_or(0);
        match first {
            b'1' => parse_ordinary(record, &mut out),
            b'2' => {
                // type 2 后面紧跟一条 NUL 分隔的 old_path
                let old_path = iter.next().map(decode_path);
                parse_rename(record, old_path, &mut out);
            }
            b'?' => parse_untracked(record, &mut out),
            b'u' => parse_unmerged(record, &mut out),
            _ => {} // ignored / unknown
        }
    }
    Ok(out)
}

fn parse_ordinary(record: &[u8], out: &mut Vec<FileStatus>) {
    // 格式："1 XY sub mH mI mW hH hI path"
    let s = std::str::from_utf8(record).unwrap_or_default();
    let mut parts = s.splitn(9, ' ');
    parts.next(); // "1"
    let xy = parts.next().unwrap_or("  ");
    for _ in 0..6 {
        parts.next();
    }
    let path = parts.next().unwrap_or("").to_string();
    let (staged, unstaged) = parse_xy(xy);
    if path.is_empty() {
        return;
    }
    out.push(FileStatus {
        path,
        old_path: None,
        staged,
        unstaged,
    });
}

fn parse_rename(record: &[u8], old_path: Option<String>, out: &mut Vec<FileStatus>) {
    // 格式："2 XY sub mH mI mW hH hI Xscore newpath"
    let s = std::str::from_utf8(record).unwrap_or_default();
    let mut parts = s.splitn(10, ' ');
    parts.next(); // "2"
    let xy = parts.next().unwrap_or("  ");
    for _ in 0..7 {
        parts.next();
    }
    let path = parts.next().unwrap_or("").to_string();
    let (staged, unstaged) = parse_xy(xy);
    if path.is_empty() {
        return;
    }
    out.push(FileStatus {
        path,
        old_path,
        staged,
        unstaged,
    });
}

fn parse_untracked(record: &[u8], out: &mut Vec<FileStatus>) {
    // 格式："? path"
    let s = std::str::from_utf8(record).unwrap_or_default();
    let path = s.strip_prefix("? ").unwrap_or(s).to_string();
    if path.is_empty() {
        return;
    }
    out.push(FileStatus {
        path,
        old_path: None,
        staged: None,
        unstaged: Some(FileChangeKind::Untracked),
    });
}

fn parse_unmerged(record: &[u8], out: &mut Vec<FileStatus>) {
    // 格式："u XY sub m1 m2 m3 mW h1 h2 h3 path"
    let s = std::str::from_utf8(record).unwrap_or_default();
    let mut parts = s.splitn(11, ' ');
    parts.next(); // "u"
    parts.next(); // XY (一般为 "UU")
    for _ in 0..8 {
        parts.next();
    }
    let path = parts.next().unwrap_or("").to_string();
    if path.is_empty() {
        return;
    }
    out.push(FileStatus {
        path,
        old_path: None,
        staged: Some(FileChangeKind::Conflicted),
        unstaged: Some(FileChangeKind::Conflicted),
    });
}

fn parse_xy(xy: &str) -> (Option<FileChangeKind>, Option<FileChangeKind>) {
    let mut chars = xy.chars();
    let x = chars.next().unwrap_or(' ');
    let y = chars.next().unwrap_or(' ');
    (code_to_kind(x), code_to_kind(y))
}

fn code_to_kind(c: char) -> Option<FileChangeKind> {
    match c {
        ' ' | '.' => None,
        'M' => Some(FileChangeKind::Modified),
        'A' => Some(FileChangeKind::Added),
        'D' => Some(FileChangeKind::Deleted),
        'R' => Some(FileChangeKind::Renamed),
        'C' => Some(FileChangeKind::Copied),
        'T' => Some(FileChangeKind::TypeChanged),
        'U' => Some(FileChangeKind::Conflicted),
        '?' => Some(FileChangeKind::Untracked),
        _ => None,
    }
}

fn decode_path(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// 当前分支领先 / 落后 upstream 的 commit 数
///
/// 无 upstream 时返回 (None, None)；有 upstream 时返回 (Some(ahead), Some(behind))
fn count_ahead_behind(repo_path: &Path) -> (Option<usize>, Option<usize>) {
    let out = match run_git_text(
        repo_path,
        &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
    ) {
        Ok(s) => s,
        Err(_) => return (None, None), // 多半是无 upstream，正常
    };
    let parts: Vec<&str> = out.split_whitespace().collect();
    if parts.len() != 2 {
        return (None, None);
    }
    (parts[0].parse().ok(), parts[1].parse().ok())
}

fn detect_operation(repo: &gix::Repository) -> Option<RepoOperation> {
    let git_dir = repo.git_dir();
    if git_dir.join("MERGE_HEAD").exists() {
        return Some(RepoOperation::Merge);
    }
    if git_dir.join("rebase-merge").is_dir() || git_dir.join("rebase-apply").is_dir() {
        return Some(RepoOperation::Rebase);
    }
    if git_dir.join("CHERRY_PICK_HEAD").exists() {
        return Some(RepoOperation::CherryPick);
    }
    if git_dir.join("REVERT_HEAD").exists() {
        return Some(RepoOperation::Revert);
    }
    None
}

fn short_branch_name(full: &gix::bstr::BStr) -> String {
    let s = full.to_string();
    s.strip_prefix("refs/heads/")
        .or_else(|| s.strip_prefix("refs/remotes/"))
        .map(|x| x.to_string())
        .unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xy_codes() {
        assert_eq!(code_to_kind(' '), None);
        assert_eq!(code_to_kind('.'), None);
        assert_eq!(code_to_kind('M'), Some(FileChangeKind::Modified));
        assert_eq!(code_to_kind('A'), Some(FileChangeKind::Added));
        assert_eq!(code_to_kind('?'), Some(FileChangeKind::Untracked));
    }
}
