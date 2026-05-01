//! gix 各模块错误 → DomainError 映射
//!
//! gix 的错误是按 sub-crate 分散的（gix-open / gix-status / gix-diff 等）。
//! 本模块统一映射成 [`DomainError`]，附中文消息

use ramag_domain::error::DomainError;

pub fn map_open_error(e: gix::open::Error) -> DomainError {
    DomainError::InvalidConfig(format!("打开仓库失败: {e}"))
}

pub fn map_status_error(e: impl std::fmt::Display) -> DomainError {
    DomainError::QueryFailed(format!("查询工作区状态失败: {e}"))
}

pub fn map_branch_error(e: impl std::fmt::Display) -> DomainError {
    DomainError::QueryFailed(format!("分支查询失败: {e}"))
}

pub fn map_log_error(e: impl std::fmt::Display) -> DomainError {
    DomainError::QueryFailed(format!("提交历史查询失败: {e}"))
}

pub fn map_diff_error(e: impl std::fmt::Display) -> DomainError {
    DomainError::QueryFailed(format!("Diff 计算失败: {e}"))
}

/// 把 git subprocess 的 stderr 翻译成中文 + 给下一步建议
///
/// 命中常见错误模式（远程不可达 / 凭证 / 锁冲突 / 冲突 / 工作区脏 / no upstream 等）时
/// 给出友好提示；未命中时返回原始 stderr，避免吞错。
///
/// # 示例
/// ```
/// use ramag_infra_git::errors::friendly_git_error;
/// let msg = friendly_git_error(
///     &["push", "origin", "main"],
///     "fatal: 'origin' does not appear to be a git repository\n",
/// );
/// assert!(msg.contains("远程仓库不可达"));
/// ```
pub fn friendly_git_error(args: &[&str], stderr: &str) -> String {
    let raw = stderr.trim();
    let cmd = args.join(" ");
    let lower = raw.to_lowercase();

    // 顺序敏感：先匹配长且具体的模式
    let hint: Option<&str> = if lower.contains("does not appear to be a git repository")
        || lower.contains("could not read from remote repository")
    {
        Some(
            "远程仓库不可达：\n\
             - 检查 remote 是否配置（git remote -v）\n\
             - 检查网络连接和访问权限",
        )
    } else if lower.contains("authentication failed")
        || lower.contains("could not read username")
        || lower.contains("permission denied (publickey)")
    {
        Some(
            "凭证错误：\n\
             - HTTPS：检查用户名 / Personal Access Token\n\
             - SSH：检查 ~/.ssh 下 key 是否加入 ssh-agent",
        )
    } else if lower.contains("merge conflict")
        || lower.contains("conflict") && lower.contains("automatic merge failed")
    {
        Some(
            "存在合并冲突：\n\
             - 工作区文件分组里会出现「冲突」段\n\
             - 用 [Use Ours]/[Use Theirs] 一键采纳，或手改后点 [✓ 标记已解决]\n\
             - 全部解决后点顶部横幅的 [继续]",
        )
    } else if lower.contains("local changes")
        && (lower.contains("would be overwritten") || lower.contains("commit your changes"))
    {
        Some(
            "工作区有未提交改动会被覆盖：\n\
             - 先 commit 这些改动，或\n\
             - 先 Stash 当前改动（左侧 sidebar Stash 段）",
        )
    } else if lower.contains("not possible to fast-forward") || lower.contains("non-fast-forward") {
        Some(
            "无法 fast-forward：\n\
             - 远程已有新 commit 而你的本地分支也有新 commit\n\
             - 改用 Pull（会创建 merge commit）或先 fetch 后 rebase",
        )
    } else if lower.contains("nothing to commit") {
        Some("暂存区为空，没有内容可提交。先 Stage 文件或勾选要提交的行。")
    } else if lower.contains("no upstream branch")
        || lower.contains("no tracking information")
        || lower.contains("the current branch") && lower.contains("has no upstream")
    {
        Some(
            "当前分支没有上游（upstream）：\n\
             - 第一次 push 会自动加 -u 参数设置 upstream\n\
             - 或在终端运行 git push -u origin <branch>",
        )
    } else if lower.contains("cannot lock ref")
        || lower.contains("unable to create") && lower.contains(".lock")
    {
        Some(
            "Git 锁文件冲突：\n\
             - 可能有其他 git 操作在进行（IDE / CLI）\n\
             - 等待对方完成；或手动删除 .git/index.lock（仅在确认无并发时）",
        )
    } else if lower.contains("bad revision") || lower.contains("unknown revision") {
        Some("找不到指定的分支 / commit / tag —— 检查名字拼写是否正确（区分大小写）")
    } else if lower.contains("ambiguous") {
        Some("名字有歧义（同名的分支和 tag），请用全名（如 refs/heads/main）")
    } else if lower.contains("would clobber existing tag") {
        Some("Tag 已存在，无法覆盖。先删除旧 tag 或换个名字。")
    } else if lower.contains("not something we can merge") {
        Some("指定的对象不能被合并（可能是 tag / commit hash 而非分支名）")
    } else if lower.contains("you have unmerged paths") {
        Some(
            "存在未解决的冲突文件：\n\
             - 工作区「冲突」段处理完所有文件\n\
             - 然后点顶部横幅 [继续]",
        )
    } else {
        None
    };

    match hint {
        Some(h) => format!("{h}\n\n[原始错误] git {cmd}: {raw}"),
        None => format!("git {cmd} 失败: {raw}"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn translates_remote_unreachable() {
        let msg = friendly_git_error(
            &["push", "origin", "main"],
            "fatal: 'origin' does not appear to be a git repository\n",
        );
        assert!(msg.contains("远程仓库不可达"));
        assert!(msg.contains("git remote -v"));
    }

    #[test]
    fn translates_no_upstream() {
        let msg = friendly_git_error(
            &["push"],
            "fatal: The current branch foo has no upstream branch.\n",
        );
        assert!(msg.contains("upstream"));
        assert!(msg.contains("-u"));
    }

    #[test]
    fn translates_merge_conflict() {
        let msg = friendly_git_error(
            &["merge", "feature"],
            "Auto-merging file.txt\nCONFLICT (content): Merge conflict in file.txt\n",
        );
        assert!(msg.contains("合并冲突"));
    }

    #[test]
    fn unknown_pattern_returns_raw() {
        let msg = friendly_git_error(&["foo"], "some weird error");
        assert!(msg.contains("git foo 失败"));
        assert!(msg.contains("some weird error"));
    }

    #[test]
    fn translates_local_changes_overwrite() {
        let msg = friendly_git_error(
            &["pull"],
            "error: Your local changes to the following files would be overwritten by merge",
        );
        assert!(msg.contains("Stash"));
    }
}
