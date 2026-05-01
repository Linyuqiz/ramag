//! Interactive rebase：生成初始计划 + 执行用户编辑后的 todos
//!
//! # 策略
//!
//! - `plan`：用 `git log --format=%H %s --reverse <onto>..HEAD` 取 commit 列表，全部标 Pick
//! - `execute`：把 todos 写到临时 shell 脚本作为 GIT_SEQUENCE_EDITOR，再 `git rebase -i <onto>`
//!   git 调用 SEQUENCE_EDITOR 时把 todo 文件路径作为 $1 传入，脚本 `cp` 我们的内容写进去

use std::path::Path;

use ramag_domain::entities::{RebaseAction, RebaseTodo};
use ramag_domain::error::{DomainError, Result};

use crate::git_cmd::run_git_text;

/// 生成 interactive rebase 初始计划（onto..HEAD，最老在前，全部 Pick）
pub fn plan(repo_path: &Path, onto: &str) -> Result<Vec<RebaseTodo>> {
    let out = run_git_text(
        repo_path,
        &[
            "log",
            "--format=%H %s",
            "--reverse",
            &format!("{onto}..HEAD"),
        ],
    )?;
    let mut todos = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((hash, subject)) = line.split_once(' ') {
            todos.push(RebaseTodo {
                action: RebaseAction::Pick,
                hash: hash.to_string(),
                subject: subject.to_string(),
            });
        }
    }
    Ok(todos)
}

/// 执行 interactive rebase：把 todos 注入到 git rebase -i
///
/// 通过临时 shell 脚本作为 GIT_SEQUENCE_EDITOR，避免弹出 $EDITOR
pub fn execute(repo_path: &Path, onto: &str, todos: &[RebaseTodo]) -> Result<()> {
    let todo_content: String = todos
        .iter()
        .map(|t| format!("{} {} {}\n", t.action.as_str(), t.short_hash(), t.subject))
        .collect();

    let tag = nano_id();
    let tmp_todo = std::env::temp_dir().join(format!("ramag_rebase_{tag}.txt"));
    std::fs::write(&tmp_todo, &todo_content)
        .map_err(|e| DomainError::Other(format!("写 rebase todo 失败: {e}")))?;

    let todo_path_str = tmp_todo
        .to_str()
        .ok_or_else(|| DomainError::Other("临时路径含非 UTF-8".into()))?;
    let script = format!("#!/bin/sh\ncp '{}' \"$1\"\n", todo_path_str);
    let tmp_script = std::env::temp_dir().join(format!("ramag_seq_editor_{tag}.sh"));
    std::fs::write(&tmp_script, &script)
        .map_err(|e| DomainError::Other(format!("写 sequence editor 脚本失败: {e}")))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&tmp_script) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&tmp_script, perms);
        }
    }

    let script_str = tmp_script
        .to_str()
        .ok_or_else(|| DomainError::Other("脚本路径含非 UTF-8".into()))?
        .to_string();

    let output = std::process::Command::new("git")
        .args(["rebase", "-i", onto])
        .env("GIT_SEQUENCE_EDITOR", &script_str)
        .env("GIT_EDITOR", "true")
        .current_dir(repo_path)
        .output()
        .map_err(|e| DomainError::Other(format!("执行 git rebase -i 失败: {e}")))?;

    let _ = std::fs::remove_file(&tmp_todo);
    let _ = std::fs::remove_file(&tmp_script);

    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() || stderr.contains("CONFLICT") || stderr.contains("conflict") {
        Ok(())
    } else {
        Err(DomainError::QueryFailed(crate::errors::friendly_git_error(
            &["rebase", "-i", onto],
            &stderr,
        )))
    }
}

fn nano_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{ns:08x}")
}
