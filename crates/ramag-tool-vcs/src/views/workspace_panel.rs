//! 工作区文件分组渲染（IDE Files 面板的内容部分）
//!
//! 提供 render_file_groups / render_group / render_file_row + bulk_op_button。
//! 主入口（含 commit panel 与 diff）改由 ide_layout 组合。

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use ramag_domain::entities::{FileChangeKind, FileStatus};

use super::helpers::{FileOp, GroupKind, code_letter_color, code_to_letter, file_op_button};
use super::vcs_view::VcsView;
use super::workspace_conflict::conflict_buttons;

impl VcsView {
    /// 工作区文件分组：已暂存 / 未暂存 / 未跟踪 / 冲突
    pub(super) fn render_file_groups(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let accent = theme.accent;
        let danger = theme.danger;

        let Some(status) = &self.status else {
            return div().into_any_element();
        };

        // 文件路径过滤关键词（来自 ide_layout 顶部搜索框，空 = 不过滤）
        let query = self
            .files_search_input
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let path_match = |p: &str| query.is_empty() || p.to_lowercase().contains(&query);

        let mut staged: Vec<FileStatus> = Vec::new();
        let mut unstaged: Vec<FileStatus> = Vec::new();
        let mut untracked: Vec<FileStatus> = Vec::new();
        let mut conflicted: Vec<FileStatus> = Vec::new();
        for f in &status.files {
            if !path_match(&f.path) {
                continue;
            }
            if f.is_conflicted() {
                conflicted.push(f.clone());
                continue;
            }
            if f.staged.is_some() {
                staged.push(f.clone());
            }
            match f.unstaged {
                Some(FileChangeKind::Untracked) => untracked.push(f.clone()),
                Some(_) => unstaged.push(f.clone()),
                None => {}
            }
        }

        if staged.is_empty() && unstaged.is_empty() && untracked.is_empty() && conflicted.is_empty()
        {
            let msg = if query.is_empty() {
                "✓ 工作区干净，无任何变更"
            } else {
                "（无匹配的变更文件，试着修改搜索关键词）"
            };
            return div()
                .px(px(2.0))
                .py(px(8.0))
                .text_sm()
                .text_color(muted_fg)
                .child(msg)
                .into_any_element();
        }

        let warm_orange = gpui::hsla(40.0 / 360.0, 0.7, 0.55, 1.0);
        let mut col = v_flex().gap(px(12.0));
        if !conflicted.is_empty() {
            col = col.child(self.render_group("冲突", danger, conflicted, GroupKind::Conflict, cx));
        }
        if !staged.is_empty() {
            col = col.child(self.render_group("已暂存", accent, staged, GroupKind::Staged, cx));
        }
        if !unstaged.is_empty() {
            col = col.child(self.render_group(
                "未暂存",
                warm_orange,
                unstaged,
                GroupKind::Unstaged,
                cx,
            ));
        }
        if !untracked.is_empty() {
            col = col.child(self.render_group(
                "未跟踪",
                muted_fg,
                untracked,
                GroupKind::Untracked,
                cx,
            ));
        }
        col.into_any_element()
    }

    /// 一组文件（如「已暂存」「未暂存」），含 header 计数 + 全组操作 + 文件行
    pub(super) fn render_group(
        &self,
        title: &'static str,
        badge_color: gpui::Hsla,
        files: Vec<FileStatus>,
        kind: GroupKind,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let count = files.len();
        let busy = self.busy;

        let mut badge_bg = badge_color;
        badge_bg.a = 0.14;

        // 按组提供"全部 stage / unstage"按钮——批量操作不再要求逐个点击
        let group_paths: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
        let bulk_btn: Option<AnyElement> = match kind {
            GroupKind::Unstaged | GroupKind::Untracked if !group_paths.is_empty() => {
                Some(bulk_op_button(
                    "stage-all",
                    title,
                    "全部 Stage",
                    FileOp::Stage,
                    IconName::Plus,
                    group_paths,
                    busy,
                    cx,
                ))
            }
            GroupKind::Staged if !group_paths.is_empty() => Some(bulk_op_button(
                "unstage-all",
                title,
                "全部 Unstage",
                FileOp::Unstage,
                IconName::Minus,
                group_paths,
                busy,
                cx,
            )),
            _ => None,
        };

        let mut header = h_flex()
            .gap(px(8.0))
            .items_center()
            .child(
                div()
                    .px(px(8.0))
                    .py(px(2.0))
                    .rounded(px(4.0))
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(badge_color)
                    .bg(badge_bg)
                    .child(title),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(muted_fg)
                    .child(format!("{count} 个文件")),
            );
        if let Some(btn) = bulk_btn {
            header = header.child(btn);
        }

        // 文件按目录树展示（中间空目录压缩，IDEA 风格）：build_tree → flatten → 渲染 dir/file
        let tree = super::file_tree::build_tree(&files);
        let mut tree_rows: Vec<super::file_tree::Row> = Vec::with_capacity(files.len() * 2);
        super::file_tree::flatten(&tree, 0, "", &self.changes_collapsed_dirs, &mut tree_rows);
        let mut body = v_flex().pl(px(2.0));
        for (i, r) in tree_rows.into_iter().enumerate() {
            body = body.child(self.render_changes_tree_row(i, &r, &files, kind, cx));
        }

        v_flex()
            .gap(px(6.0))
            .child(header)
            .child(body)
            .into_any_element()
    }

    /// Changes 树行：dir 行（▾▸ + 名 + 计数，可折叠）/ file 行（复用 render_file_row + 缩进）
    fn render_changes_tree_row(
        &self,
        idx: usize,
        row: &super::file_tree::Row,
        files: &[FileStatus],
        kind: GroupKind,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted_fg = theme.muted_foreground;
        let hover_bg = theme.muted;
        match row {
            super::file_tree::Row::Dir {
                display_name,
                dir_path,
                depth,
                is_collapsed,
                file_count,
            } => {
                let id = SharedString::from(format!("vcs-ch-dir-{idx}-{dir_path}"));
                let icon = if *is_collapsed { "▸" } else { "▾" };
                let dir_clone = dir_path.clone();
                h_flex()
                    .id(id)
                    .gap(px(4.0))
                    .items_center()
                    .py(px(2.0))
                    .pr(px(6.0))
                    .pl(px((4 + depth * 12) as f32))
                    .rounded(px(3.0))
                    .cursor_pointer()
                    .hover(move |this| this.bg(hover_bg))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.toggle_changes_dir(dir_clone.clone(), cx);
                    }))
                    .child(
                        div()
                            .flex_none()
                            .w(px(12.0))
                            .text_xs()
                            .text_color(muted_fg)
                            .child(icon),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(fg)
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(display_name.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(format!("{file_count}")),
                    )
                    .into_any_element()
            }
            super::file_tree::Row::File { idx: f_idx, depth } => {
                let f = files[*f_idx].clone();
                let inner = self.render_file_row(idx, f, kind, cx).into_any_element();
                div()
                    .pl(px((depth * 12) as f32))
                    .child(inner)
                    .into_any_element()
            }
        }
    }

    /// 切换 Changes 文件树某目录的折叠状态
    pub(super) fn toggle_changes_dir(&mut self, dir_path: String, cx: &mut Context<Self>) {
        if !self.changes_collapsed_dirs.remove(&dir_path) {
            self.changes_collapsed_dirs.insert(dir_path);
        }
        cx.notify();
    }

    /// 单文件行：变更字母 + 路径 + 行尾按钮；整行可点击查看 diff
    pub(super) fn render_file_row(
        &self,
        idx: usize,
        f: FileStatus,
        kind: GroupKind,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted_fg = theme.muted_foreground;
        let hover_bg = theme.muted;
        let mut selected_bg = theme.accent;
        selected_bg.a = 0.16;

        let code_kind = match kind {
            GroupKind::Staged => f.staged,
            GroupKind::Unstaged | GroupKind::Untracked => f.unstaged,
            GroupKind::Conflict => f.staged.or(f.unstaged),
        };
        let code = code_to_letter(code_kind);
        let code_color = code_letter_color(code, muted_fg);

        let path_label = match (&f.old_path, &f.path) {
            (Some(old), new) if old != new => format!("{old} → {new}"),
            _ => f.path.clone(),
        };

        let path_for_buttons = f.path.clone();
        let path_for_click = f.path.clone();
        let busy = self.busy;
        let is_selected = self
            .selected_file
            .as_ref()
            .map(|(p, k)| p == &f.path && *k == kind)
            .unwrap_or(false);
        let buttons: Vec<AnyElement> = match kind {
            GroupKind::Staged => vec![file_op_button(
                ("unstage", idx, &f.path),
                "Unstage",
                FileOp::Unstage,
                path_for_buttons.clone(),
                busy,
                cx,
            )],
            GroupKind::Unstaged => vec![
                file_op_button(
                    ("stage", idx, &f.path),
                    "Stage",
                    FileOp::Stage,
                    path_for_buttons.clone(),
                    busy,
                    cx,
                ),
                file_op_button(
                    ("discard", idx, &f.path),
                    "丢弃",
                    FileOp::Discard,
                    path_for_buttons.clone(),
                    busy,
                    cx,
                ),
            ],
            GroupKind::Untracked => vec![file_op_button(
                ("stage-u", idx, &f.path),
                "Stage",
                FileOp::Stage,
                path_for_buttons.clone(),
                busy,
                cx,
            )],
            GroupKind::Conflict => conflict_buttons(idx, &f.path, busy, cx),
        };

        // 「查看历史」按钮：所有非 Untracked 文件都可看（untracked 文件还没进 git，无历史）
        let history_btn: Option<AnyElement> = if matches!(kind, GroupKind::Untracked) {
            None
        } else {
            let path_for_history = f.path.clone();
            let id = SharedString::from(format!("vcs-file-history-{idx}-{}", f.path));
            Some(
                Button::new(id)
                    .ghost()
                    .xsmall()
                    .icon(ramag_ui::icons::scroll_text())
                    .tooltip("查看此文件的历史")
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.view_file_history(path_for_history.clone(), cx);
                    }))
                    .into_any_element(),
            )
        };
        let mut buttons = buttons;
        if let Some(b) = history_btn {
            buttons.insert(0, b);
        }

        let row_id = SharedString::from(format!("vcs-file-{}-{}-{:?}", idx, f.path, kind));
        let mut row = h_flex()
            .id(row_id)
            .gap(px(8.0))
            .items_center()
            .py(px(2.0))
            .px(px(4.0))
            .rounded(px(3.0))
            .cursor_pointer()
            .hover(move |this| this.bg(hover_bg))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.select_file(path_for_click.clone(), kind, cx);
            }))
            .child(
                div()
                    .w(px(14.0))
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(code_color)
                    .child(code),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(fg)
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(path_label),
            )
            .child(
                h_flex()
                    .gap(px(4.0))
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .children(buttons),
            );
        if is_selected {
            row = row.bg(selected_bg);
        }
        row
    }
}

/// 「全部 Stage」「全部 Unstage」按钮：同组所有文件批量执行 file_op
#[allow(clippy::too_many_arguments)]
fn bulk_op_button(
    kind: &'static str,
    title: &'static str,
    label: &'static str,
    op: FileOp,
    icon: IconName,
    paths: Vec<String>,
    busy: bool,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let id = SharedString::from(format!("vcs-bulk-{kind}-{title}"));
    Button::new(id)
        .ghost()
        .xsmall()
        .icon(icon)
        .label(label)
        .disabled(busy)
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            for p in paths.clone() {
                this.run_file_op(op, p, cx);
            }
        }))
        .into_any_element()
}
