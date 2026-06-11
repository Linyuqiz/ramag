//! hunk 级 patch：discard_hunk（按 source 分流回滚）+ build_patch_for_hunk

use gpui::Context;
use ramag_domain::entities::{DiffLineKind, FileDiff};
use tracing::{error, info};

use super::helpers::{FileTabSource, GroupKind};
use super::vcs_view::VcsView;

impl VcsView {
    /// 回滚 hunk：Unstaged 走 discard_patch（reverse 到 index）/ Staged 走 unstage_patch（reverse 撤回工作区）。
    /// 失败常因 diff 拉取后工作区或 index 又改过，patch 上下文不匹配
    pub(super) fn discard_hunk(&mut self, hunk_idx: usize, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let Some(diff) = self.current_diff.clone() else {
            return;
        };
        // 仅 Changes 文件的 hunk 可回滚；其他来源（Commit detail / ProjectFiles）UI 不应渲染该按钮
        let kind = self
            .active_file_tab_idx
            .and_then(|i| self.file_tabs.get(i))
            .and_then(|t| match &t.source {
                FileTabSource::Changes(k) => Some(*k),
                _ => None,
            });
        let Some(kind) = kind else {
            self.error = Some("当前不是 Changes diff，无法回滚".into());
            cx.notify();
            return;
        };
        if !matches!(kind, GroupKind::Staged | GroupKind::Unstaged) {
            // Untracked / Conflict diff 在 render_diff_body 里就被替换为 placeholder，
            // 不会渲染 hunk header，所以理论到不了这里；保险起见兜底
            self.error = Some("此类文件不支持 hunk 回滚".into());
            cx.notify();
            return;
        }
        let Some(patch) = build_patch_for_hunk(&diff, hunk_idx) else {
            self.error = Some("hunk 索引越界".into());
            cx.notify();
            return;
        };
        let driver = self.driver.clone();
        self.busy = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = match kind {
                GroupKind::Staged => driver.unstage_patch(&repo, &patch).await,
                GroupKind::Unstaged => driver.discard_patch(&repo, &patch).await,
                _ => unreachable!("已在前置分支拦截"),
            };
            let new_status = driver.status(&repo).await.ok();
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.busy_label = None;
                if !this.is_current_repo(&repo) {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(()) => {
                        info!(hunk_idx, ?kind, "vcs: hunk revert done");
                        if let Some(s) = new_status {
                            this.status = Some(s);
                        }
                        // tabs 对齐：同文件两组 tab 缓存一并失效；变更全回滚则关 tab；active 自动重拉
                        this.sync_changes_tabs_with_status(cx);
                    }
                    Err(e) => {
                        error!(error = %e, hunk_idx, ?kind, "vcs: hunk revert failed");
                        let action = match kind {
                            GroupKind::Staged => "撤回 hunk 到工作区",
                            GroupKind::Unstaged => "回滚 hunk 到 index",
                            _ => "回滚 hunk",
                        };
                        this.error = Some(format!("{action} 失败：{e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// 整个 hunk（含 context + `+/-` 全部行）→ unified diff patch，给 hunk 回滚用
pub(super) fn build_patch_for_hunk(diff: &FileDiff, hunk_idx: usize) -> Option<String> {
    let hunk = diff.hunks.get(hunk_idx)?;
    let mut out = String::new();
    let path = &diff.path;
    out.push_str(&format!("diff --git a/{path} b/{path}\n"));
    out.push_str(&format!("--- a/{path}\n"));
    out.push_str(&format!("+++ b/{path}\n"));
    // 占位 hunk header；--recount 让 git 自动重算 line counts
    out.push_str("@@ -1,1 +1,1 @@\n");
    for line in &hunk.lines {
        let prefix = match line.kind {
            DiffLineKind::Context => " ",
            DiffLineKind::Add => "+",
            DiffLineKind::Delete => "-",
        };
        out.push_str(&format!("{prefix}{}\n", line.text));
    }
    Some(out)
}
