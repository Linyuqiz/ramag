//! VcsView 行级 patch 操作
//!
//! 与文件级 stage / unstage 并列：
//! - **toggle_diff_line**：用户在 diff_panel 点 +/- 行 → 切换 selected_diff_lines
//! - **stage_selected_lines**：把当前选中的 +/- 行构造成 unified patch 文本，
//!   调 driver.stage_patch 写入暂存区；成功后清空选中并刷新
//!
//! patch 文本约定：
//! - 文件头三行（diff --git / --- / +++）
//! - 每个 *有选中* 的 hunk 输出 `@@ -1,1 +1,1 @@`（占位行号）+ 行
//!   - context 行：原样保留为 " text"
//!   - 选中的 add：保留为 "+text"
//!   - 未选中的 add：跳过（patch 里不出现 → apply 时不写入 index）
//!   - 选中的 delete：保留为 "-text"
//!   - 未选中的 delete：降级为 context " text"（patch 视为未删除）
//! - 配合 `git apply --cached --recount --unidiff-zero` 自动重算行号

use std::collections::HashSet;

use gpui::Context;
use ramag_domain::entities::{DiffLineKind, FileDiff};
use tracing::{error, info};

use super::helpers::{FileTabSource, GroupKind};
use super::vcs_view::VcsView;

#[allow(dead_code)]
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

    /// 把当前选中行 stage 进暂存区（diff 来源 = Unstaged 时使用）
    pub(super) fn stage_selected_lines(&mut self, cx: &mut Context<Self>) {
        self.run_patch_op(false, cx);
    }

    /// 把当前选中行从暂存区撤回（diff 来源 = Staged 时使用；调 unstage_patch）
    pub(super) fn unstage_selected_lines(&mut self, cx: &mut Context<Self>) {
        self.run_patch_op(true, cx);
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

    /// 共用：构造 patch + 调 driver；reverse=true 走 unstage_patch
    fn run_patch_op(&mut self, reverse: bool, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let Some(diff) = self.current_diff.clone() else {
            return;
        };
        let Some(patch) = build_patch_from_selection(&diff, &self.selected_diff_lines) else {
            self.error = Some("未选中任何要操作的行".into());
            cx.notify();
            return;
        };
        let driver = self.driver.clone();
        self.busy = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = if reverse {
                driver.unstage_patch(&repo, &patch).await
            } else {
                driver.stage_patch(&repo, &patch).await
            };
            let new_status = driver.status(&repo).await.ok();
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        info!(reverse, "vcs: patch op done");
                        this.selected_diff_lines.clear();
                        if let Some(s) = new_status {
                            this.status = Some(s);
                        }
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
                        error!(error = %e, reverse, "vcs: patch op failed");
                        let kind = if reverse { "Unstage" } else { "Stage" };
                        this.error = Some(format!("行级 {kind} 失败：{e}"));
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

/// 把 (FileDiff + 选中行集合) 构造成 unified diff patch
///
/// 没有任何选中行时返回 None；调用方应弹错或忽略
pub(super) fn build_patch_from_selection(
    diff: &FileDiff,
    selected: &HashSet<(usize, usize)>,
) -> Option<String> {
    if selected.is_empty() {
        return None;
    }
    let mut out = String::new();
    let path = &diff.path;
    out.push_str(&format!("diff --git a/{path} b/{path}\n"));
    out.push_str(&format!("--- a/{path}\n"));
    out.push_str(&format!("+++ b/{path}\n"));
    let mut wrote_any = false;
    for (hi, hunk) in diff.hunks.iter().enumerate() {
        let has_selection = hunk
            .lines
            .iter()
            .enumerate()
            .any(|(li, l)| l.kind != DiffLineKind::Context && selected.contains(&(hi, li)));
        if !has_selection {
            continue;
        }
        // 占位 hunk header；--recount 让 git 自动重算 line counts
        out.push_str("@@ -1,1 +1,1 @@\n");
        for (li, line) in hunk.lines.iter().enumerate() {
            let sel = selected.contains(&(hi, li));
            match line.kind {
                DiffLineKind::Context => {
                    out.push_str(&format!(" {}\n", line.text));
                }
                DiffLineKind::Add => {
                    if sel {
                        out.push_str(&format!("+{}\n", line.text));
                    }
                }
                DiffLineKind::Delete => {
                    if sel {
                        out.push_str(&format!("-{}\n", line.text));
                    } else {
                        out.push_str(&format!(" {}\n", line.text));
                    }
                }
            }
        }
        wrote_any = true;
    }
    if wrote_any { Some(out) } else { None }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ramag_domain::entities::{DiffLine, DiffLineKind, FileChangeKind, FileDiff, Hunk};

    fn mk_diff() -> FileDiff {
        FileDiff {
            path: "src/lib.rs".into(),
            old_path: None,
            change_kind: FileChangeKind::Modified,
            binary: false,
            old_mode: None,
            new_mode: None,
            hunks: vec![Hunk {
                old_start: 1,
                old_lines: 3,
                new_start: 1,
                new_lines: 3,
                heading: None,
                lines: vec![
                    DiffLine {
                        kind: DiffLineKind::Context,
                        old_lineno: Some(1),
                        new_lineno: Some(1),
                        text: "a".into(),
                    },
                    DiffLine {
                        kind: DiffLineKind::Delete,
                        old_lineno: Some(2),
                        new_lineno: None,
                        text: "old".into(),
                    },
                    DiffLine {
                        kind: DiffLineKind::Add,
                        old_lineno: None,
                        new_lineno: Some(2),
                        text: "new".into(),
                    },
                    DiffLine {
                        kind: DiffLineKind::Context,
                        old_lineno: Some(3),
                        new_lineno: Some(3),
                        text: "c".into(),
                    },
                ],
            }],
        }
    }

    #[test]
    fn empty_selection_returns_none() {
        let diff = mk_diff();
        let sel: HashSet<(usize, usize)> = HashSet::new();
        assert!(build_patch_from_selection(&diff, &sel).is_none());
    }

    #[test]
    fn selecting_only_add_skips_delete() {
        let diff = mk_diff();
        let mut sel = HashSet::new();
        sel.insert((0, 2)); // Add 行
        let p = build_patch_from_selection(&diff, &sel).unwrap();
        // 文件头
        assert!(p.contains("diff --git a/src/lib.rs b/src/lib.rs"));
        // 选中 add → +new
        assert!(p.contains("+new"));
        // 未选中 delete → 降级为 context " old"
        assert!(p.contains(" old\n"));
        // 不应保留 -old
        assert!(!p.contains("-old"));
    }

    #[test]
    fn selecting_only_delete_skips_add() {
        let diff = mk_diff();
        let mut sel = HashSet::new();
        sel.insert((0, 1)); // Delete 行
        let p = build_patch_from_selection(&diff, &sel).unwrap();
        assert!(p.contains("-old"));
        // 未选中 add → 不出现
        assert!(!p.contains("+new"));
    }
}
