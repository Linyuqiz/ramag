//! VcsView hunk 级 patch 操作
//!
//! - **toggle_diff_line**：用户在 diff_panel 点 +/- 行 → 切换 selected_diff_lines（高亮反馈）
//! - **discard_hunk**：IDEA 风格 ↶ 按钮，把整个 hunk 反向应用回 HEAD（按 source kind 分流）
//! - **build_patch_for_hunk**：把单个 hunk 构造成 unified diff patch 文本

use gpui::Context;
use ramag_domain::entities::{DiffLineKind, FileDiff};
use tracing::{error, info};

use super::helpers::{FileTabSource, GroupKind};
use super::vcs_view::VcsView;

impl VcsView {
    /// 切换某行的选中状态（hunk_idx + 行在 hunk 里的索引）
    pub(super) fn toggle_diff_line(
        &mut self,
        hunk_idx: usize,
        line_idx: usize,
        cx: &mut Context<Self>,
    ) {
        let key = (hunk_idx, line_idx);
        if !self.selected_diff_lines.insert(key) {
            self.selected_diff_lines.remove(&key);
        }
        cx.notify();
    }

    /// 回滚整个 hunk（IDEA 风格 ↶ 按钮）—— 按当前 diff 的 kind 分流不同语义：
    ///
    /// | 来源 kind     | 调用                  | 含义                                |
    /// |---------------|-----------------------|------------------------------------|
    /// | `Unstaged`    | `discard_patch`       | `git apply --reverse`，工作区回到 index 状态 |
    /// | `Staged`      | `unstage_patch`       | `git apply --cached --reverse`，把 hunk 撤回到工作区 |
    /// | `Untracked` / `Conflict` | -          | 不会进入（render_diff_body 已挡掉 diff 显示） |
    /// | `Commit` 详情 | -                      | 只读 diff，UI 层 enable_discard=false |
    ///
    /// 失败常因 diff 拉取后工作区/index 又被改了，patch 上下文不匹配
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
                match result {
                    Ok(()) => {
                        info!(hunk_idx, ?kind, "vcs: hunk revert done");
                        if let Some(s) = new_status {
                            this.status = Some(s);
                        }
                        // 清缓存让 select_file 强制重拉（回滚后内容已变，旧 cached_diff 失效）
                        if let Some(idx) = this.active_file_tab_idx
                            && let Some(tab) = this.file_tabs.get_mut(idx)
                        {
                            tab.cached_diff = None;
                        }
                        this.current_diff = None;
                        if let Some((p, k)) = this.selected_file.clone() {
                            this.select_file(p, k, cx);
                        }
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

/// 把整个 hunk（含 context / +/- 全部行）构造成 unified diff patch
///
/// 用于 hunk「↶」回滚按钮：拿这个 patch 调 driver.discard_patch（git apply --reverse）
/// 把该 hunk 的所有改动从工作区反向应用回 HEAD 状态
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
