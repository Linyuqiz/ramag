//! 工作区文件分组渲染（IDE Files 内容）。4 组（冲突/已暂存/未暂存/未跟踪）扁平为
//! 单个 uniform_list（分组表头行 + 目录行 + 文件行，全 28px 等高），万级变更也只渲染可见行

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, px, uniform_list,
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

/// 行高固定 28px：uniform_list 行级虚拟化要求所有行等高（表头 / 目录 / 文件同高）
const ROW_H: f32 = 28.0;

/// 扁平后的 Changes 行：分组表头 / 目录 / 文件
enum ChangeRow {
    Header {
        title: &'static str,
        color: gpui::Hsla,
        kind: GroupKind,
        paths: Vec<String>,
    },
    Dir {
        display_name: String,
        dir_path: String,
        depth: usize,
        is_collapsed: bool,
        file_count: usize,
    },
    File {
        file: FileStatus,
        depth: usize,
        kind: GroupKind,
    },
}

impl VcsView {
    /// 工作区文件分组：4 组扁平为单 uniform_list（分组表头行 + 目录 / 文件行）
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

        // 4 组按固定顺序扁平为一个行序列
        let warm_orange = gpui::hsla(40.0 / 360.0, 0.7, 0.55, 1.0);
        let mut rows: Vec<ChangeRow> = Vec::new();
        self.append_change_group("冲突", danger, GroupKind::Conflict, conflicted, &mut rows);
        self.append_change_group("已暂存", accent, GroupKind::Staged, staged, &mut rows);
        self.append_change_group(
            "未暂存",
            warm_orange,
            GroupKind::Unstaged,
            unstaged,
            &mut rows,
        );
        self.append_change_group(
            "未跟踪",
            muted_fg,
            GroupKind::Untracked,
            untracked,
            &mut rows,
        );

        let rows_rc: Rc<Vec<ChangeRow>> = Rc::new(rows);
        let total = rows_rc.len();
        let body = uniform_list(
            "vcs-changes-rows",
            total,
            cx.processor({
                let rows_rc = rows_rc.clone();
                move |this, range: Range<usize>, _w, cx| {
                    range
                        .map(|i| this.render_change_row(i, &rows_rc[i], cx))
                        .collect::<Vec<_>>()
                }
            }),
        )
        .track_scroll(&self.changes_scroll)
        .flex_1();

        // size_full + min_h_0：在外层 overflow_y_scrollbar 容器内拿到确定高度（同 project_files）
        v_flex()
            .size_full()
            .min_h_0()
            .child(body)
            .into_any_element()
    }

    /// 把一组文件（build_tree → flatten）追加成 Header + Dir/File 行
    fn append_change_group(
        &self,
        title: &'static str,
        color: gpui::Hsla,
        kind: GroupKind,
        files: Vec<FileStatus>,
        out: &mut Vec<ChangeRow>,
    ) {
        if files.is_empty() {
            return;
        }
        let paths: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
        out.push(ChangeRow::Header {
            title,
            color,
            kind,
            paths,
        });
        let tree = super::file_tree::build_tree(&files);
        let mut trows: Vec<super::file_tree::Row> = Vec::with_capacity(files.len() * 2);
        super::file_tree::flatten(&tree, 0, "", &self.changes_collapsed_dirs, &mut trows);
        for r in trows {
            match r {
                super::file_tree::Row::Dir {
                    display_name,
                    dir_path,
                    depth,
                    is_collapsed,
                    file_count,
                } => out.push(ChangeRow::Dir {
                    display_name,
                    dir_path,
                    depth,
                    is_collapsed,
                    file_count,
                }),
                super::file_tree::Row::File { idx, depth } => out.push(ChangeRow::File {
                    file: files[idx].clone(),
                    depth,
                    kind,
                }),
            }
        }
    }

    /// uniform_list 单行分发：表头 / 目录 / 文件
    fn render_change_row(&self, i: usize, row: &ChangeRow, cx: &mut Context<Self>) -> AnyElement {
        match row {
            ChangeRow::Header {
                title,
                color,
                kind,
                paths,
            } => self.render_change_header_row(title, *color, *kind, paths, cx),
            ChangeRow::Dir {
                display_name,
                dir_path,
                depth,
                is_collapsed,
                file_count,
            } => self.render_change_dir_row(
                i,
                display_name,
                dir_path,
                *depth,
                *is_collapsed,
                *file_count,
                cx,
            ),
            ChangeRow::File { file, depth, kind } => div()
                .w_full()
                .h(px(ROW_H))
                .flex_none()
                .pl(px((*depth as f32) * 12.0))
                .child(self.render_file_row(i, file.clone(), *kind, cx))
                .into_any_element(),
        }
    }

    /// 分组表头行：色块徽标 + 计数 + 全组批量按钮（顶边线分隔相邻组）
    fn render_change_header_row(
        &self,
        title: &'static str,
        badge_color: gpui::Hsla,
        kind: GroupKind,
        paths: &[String],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let border = theme.border;
        let busy = self.busy;
        let count = paths.len();
        let mut badge_bg = badge_color;
        badge_bg.a = 0.14;

        // 按组提供"全部 stage / unstage"批量操作
        let bulk_btn: Option<AnyElement> = match kind {
            GroupKind::Unstaged | GroupKind::Untracked if !paths.is_empty() => {
                Some(bulk_op_button(
                    "stage-all",
                    title,
                    "全部 Stage",
                    FileOp::Stage,
                    IconName::Plus,
                    paths.to_vec(),
                    busy,
                    cx,
                ))
            }
            GroupKind::Staged if !paths.is_empty() => Some(bulk_op_button(
                "unstage-all",
                title,
                "全部 Unstage",
                FileOp::Unstage,
                IconName::Minus,
                paths.to_vec(),
                busy,
                cx,
            )),
            _ => None,
        };

        let mut row = h_flex()
            .h(px(ROW_H))
            .flex_none()
            .w_full()
            .gap(px(8.0))
            .items_center()
            .border_t_1()
            .border_color(border)
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
            row = row.child(btn);
        }
        row.into_any_element()
    }

    /// 目录行：折叠图标 + 名 + 文件计数（整行可点切换折叠）
    #[allow(clippy::too_many_arguments)]
    fn render_change_dir_row(
        &self,
        i: usize,
        display_name: &str,
        dir_path: &str,
        depth: usize,
        is_collapsed: bool,
        file_count: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted_fg = theme.muted_foreground;
        let hover_bg = theme.muted;
        let id = SharedString::from(format!("vcs-ch-dir-{i}-{dir_path}"));
        let icon = if is_collapsed { "▸" } else { "▾" };
        let dir_clone = dir_path.to_string();
        h_flex()
            .id(id)
            .h(px(ROW_H))
            .flex_none()
            .w_full()
            .gap(px(4.0))
            .items_center()
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
                    .child(display_name.to_string()),
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

    /// 切换 Changes 文件树某目录的折叠状态
    pub(super) fn toggle_changes_dir(&mut self, dir_path: String, cx: &mut Context<Self>) {
        if !self.changes_collapsed_dirs.remove(&dir_path) {
            self.changes_collapsed_dirs.insert(dir_path);
        }
        cx.notify();
    }

    /// 单文件行：变更字母 + 路径 + 行尾按钮；整行可点击查看 diff（固定 28px 高，适配虚拟列表）
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
            .h(px(ROW_H))
            .flex_none()
            .w_full()
            .gap(px(8.0))
            .items_center()
            .px(px(4.0))
            .rounded(px(3.0))
            .cursor_pointer()
            .hover(move |this| this.bg(hover_bg))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                // 冲突文件：点击行直达三栏解决器（diff 区无法表达三方内容）
                if matches!(kind, GroupKind::Conflict) {
                    this.open_conflict_editor(path_for_click.clone(), cx);
                } else {
                    this.select_file(path_for_click.clone(), kind, cx);
                }
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
            this.run_file_op(op, paths.clone(), cx);
        }))
        .into_any_element()
}
