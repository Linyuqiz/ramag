//! 子进程 `git` 命令调用封装
//!
//! 写操作（stage / commit / push / pull / fetch / discard）以及部分复杂查询（status 详情、
//! diff、ahead/behind 计算）走子进程 `git`，原因：
//! - gix 写 API 还在演进期，stable 用法尚不齐
//! - 系统 git 已经处理好凭证（osxkeychain helper / SSH agent）和钩子
//! - subprocess 调用对错误的可观察性更好（stderr 直接拿到）
//!
//! 读操作（仓库元数据 / 分支列表 / log 遍历）继续走 [`gix`]，性能更好

use std::path::Path;
use std::process::Command;

use ramag_domain::error::{DomainError, Result};
use tracing::debug;

use crate::errors::friendly_git_error;

/// 在指定仓库目录跑 `git <args>`，返回 stdout 字节
///
/// 失败时把 stderr 包进 [`DomainError::QueryFailed`]，便于 UI 直接展示。
/// - `-C <repo_path>` 让 git 在该路径下定位仓库（不依赖当前工作目录）
/// - `-c core.quotepath=false` 让 git 输出原始 utf-8 路径（默认会把非 ASCII 转义成 \xxx）
pub fn run_git_bytes(repo_path: &Path, args: &[&str]) -> Result<Vec<u8>> {
    debug!(?repo_path, ?args, "git subprocess");
    let output = Command::new("git")
        .arg("-c")
        .arg("core.quotepath=false")
        .arg("-C")
        .arg(repo_path)
        .args(args)
        .output()
        .map_err(|e| DomainError::QueryFailed(format!("git 调用失败（请确认已安装 git）: {e}")))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(DomainError::QueryFailed(friendly_git_error(args, &err)));
    }
    Ok(output.stdout)
}

/// 跑 git 命令并把 stdout 解析成字符串（默认 UTF-8；非 UTF-8 用 lossy）
pub fn run_git_text(repo_path: &Path, args: &[&str]) -> Result<String> {
    let bytes = run_git_bytes(repo_path, args)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// 跑 git 命令并把 patch 文本写入 stdin
///
/// 用于 `git apply --cached`、`git am` 等需要读 stdin 的子命令
pub fn run_git_stdin(repo_path: &Path, args: &[&str], stdin_text: &str) -> Result<Vec<u8>> {
    use std::io::Write;
    use std::process::Stdio;
    debug!(
        ?repo_path,
        ?args,
        stdin_len = stdin_text.len(),
        "git subprocess (stdin)"
    );
    let mut child = Command::new("git")
        .arg("-c")
        .arg("core.quotepath=false")
        .arg("-C")
        .arg(repo_path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| DomainError::QueryFailed(format!("git 调用失败: {e}")))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_text.as_bytes())
            .map_err(|e| DomainError::QueryFailed(format!("写入 git stdin 失败: {e}")))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| DomainError::QueryFailed(format!("git 等待失败: {e}")))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(DomainError::QueryFailed(friendly_git_error(args, &err)));
    }
    Ok(output.stdout)
}
