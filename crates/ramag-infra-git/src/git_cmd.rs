//! `git` 子进程封装。写操作 + 复杂查询走 subprocess（凭证 / 钩子由系统 git 处理）；
//! 读元数据 / 分支 / log 走 gix（更快）

use std::path::Path;
use std::process::Command;

use ramag_domain::error::{DomainError, Result};
use tracing::debug;

use crate::errors::friendly_git_error;

/// `-C` 锁定仓库目录；`-c core.quotepath=false` 让非 ASCII 路径走原始 utf-8
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

/// stdout 解析成 String，非 UTF-8 走 lossy
pub fn run_git_text(repo_path: &Path, args: &[&str]) -> Result<String> {
    let bytes = run_git_bytes(repo_path, args)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// 把文本写入 stdin。`git apply --cached` / `git am` 等用
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
