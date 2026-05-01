//! Reset / Revert 操作
//!
//! 与 cherry_pick / merge / rebase 同属「改写历史 / 移动 HEAD」类操作；
//! 抽出独立模块让 lib.rs 不超 600 行。

use std::path::Path;

use ramag_domain::entities::ResetKind;
use ramag_domain::error::Result;

use crate::git_cmd::run_git_bytes;

/// 重置 HEAD 到指定 commit
pub fn reset(repo_path: &Path, target: &str, kind: ResetKind) -> Result<()> {
    let flag = match kind {
        ResetKind::Soft => "--soft",
        ResetKind::Mixed => "--mixed",
        ResetKind::Hard => "--hard",
    };
    run_git_bytes(repo_path, &["reset", flag, target]).map(|_| ())
}

/// 生成反向 commit 撤销指定 commit（不改写历史，安全）
pub fn revert(repo_path: &Path, commit: &str) -> Result<()> {
    // --no-edit 用默认 message，不弹编辑器（避免阻塞 GPUI）
    run_git_bytes(repo_path, &["revert", "--no-edit", commit]).map(|_| ())
}
