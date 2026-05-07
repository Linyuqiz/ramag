//! Commit 详情面板（左侧 sidebar）：commit metadata + 文件树（按目录组织 + 中间空目录压缩）
//!
//! 与 Changes 文件分组共用 [`super::file_tree`] 构建相同的目录结构，保证两边视觉一致

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, px, uniform_list,
};
use gpui_component::{
    ActiveTheme, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use ramag_domain::entities::{Commit, FileStatus};

use super::file_tree::{Row, build_tree, flatten};
use super::helpers::{code_letter_color, code_to_letter};
use super::vcs_view::VcsView;

impl VcsView {
    /// Commit 详情面板：close 按钮 + 简略 metadata + 文件树
    pub(super) fn render_commit_detail_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(commit) = self.viewing_commit.clone() else {
            return div().into_any_element();
        };
        let (border, muted_fg, fg, accent) = {
            let t = cx.theme();
            (t.border, t.muted_foreground, t.foreground, t.accent)
        };
        render_left_sidebar(self, &commit, fg, muted_fg, accent, border, cx)
    }

    /// 切换 commit 文件树目录的折叠状态
    pub(super) fn toggle_commit_files_dir(&mut self, dir_path: String, cx: &mut Context<Self>) {
        if !self.commit_files_collapsed.remove(&dir_path) {
            self.commit_files_collapsed.insert(dir_path);
        }
        cx.notify();
    }
}

fn render_left_sidebar(
    view: &VcsView,
    commit: &Commit,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    accent: gpui::Hsla,
    border: gpui::Hsla,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let close_btn = Button::new("vcs-commit-detail-close")
        .ghost()
        .xsmall()
        .icon(IconName::Close)
        .tooltip("关闭详情面板")
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.close_commit_detail(cx);
        }));
    let mono = cx.theme().mono_font_family.clone();
    let head = h_flex()
        .items_center()
        .gap(px(6.0))
        .h(px(36.0))
        .flex_none()
        .px(px(8.0))
        .border_b_1()
        .border_color(border)
        .child(close_btn)
        .child(
            div()
                .font_family(mono)
                .text_xs()
                .text_color(accent)
                .child(commit.id.short().to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(muted_fg)
                .overflow_hidden()
                .text_ellipsis()
                .child(format!("{} 个文件", view.commit_files.len())),
        );

    let body = render_files_tree(view, fg, muted_fg, cx);

    v_flex()
        .size_full()
        .border_l_1()
        .border_color(border)
        .child(head)
        .child(body)
        .into_any_element()
}

/// 树状文件列表：build_tree → flatten → uniform_list 行级虚拟化渲染
fn render_files_tree(
    view: &VcsView,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    if view.loading_commit_files {
        return v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(muted_fg)
            .child("加载文件列表...")
            .into_any_element();
    }
    if view.commit_files.is_empty() {
        return v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(muted_fg)
            .child("(commit 无文件变更)")
            .into_any_element();
    }
    let commit_id = view
        .viewing_commit
        .as_ref()
        .map(|c| c.id.0.clone())
        .unwrap_or_default();

    let tree = build_tree(&view.commit_files);
    let mut rows: Vec<Row> = Vec::with_capacity(view.commit_files.len() * 2);
    flatten(&tree, 0, "", &view.commit_files_collapsed, &mut rows);
    let rows_rc: Rc<Vec<Row>> = Rc::new(rows);
    let files_rc: Rc<Vec<FileStatus>> = Rc::new(view.commit_files.clone());
    let total = rows_rc.len();
    let scroll = view.commit_files_scroll.clone();
    let commit_id_rc: Rc<String> = Rc::new(commit_id);

    uniform_list(
        "vcs-commit-files",
        total,
        cx.processor({
            let rows_rc = rows_rc.clone();
            let files_rc = files_rc.clone();
            let commit_id_rc = commit_id_rc.clone();
            move |this, range: Range<usize>, _w, cx| {
                let selected = this.selected_commit_file.clone();
                range
                    .map(|i| {
                        render_tree_row(
                            i,
                            &rows_rc[i],
                            &files_rc,
                            &selected,
                            commit_id_rc.as_str(),
                            fg,
                            muted_fg,
                            cx,
                        )
                    })
                    .collect::<Vec<_>>()
            }
        }),
    )
    .track_scroll(&scroll)
    .h_full()
    .flex_1()
    .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_tree_row(
    idx_in_rows: usize,
    row: &Row,
    files: &Rc<Vec<FileStatus>>,
    selected: &Option<String>,
    commit_id: &str,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let theme = cx.theme();
    let hover_bg = theme.muted;
    let mut sel_bg = theme.accent;
    sel_bg.a = 0.16;

    match row {
        Row::Dir {
            display_name,
            dir_path,
            depth,
            is_collapsed,
            file_count,
        } => {
            let id = SharedString::from(format!("vcs-cd-dir-{idx_in_rows}-{dir_path}"));
            let icon = if *is_collapsed { "▸" } else { "▾" };
            let dir_clone = dir_path.clone();
            h_flex()
                .id(id)
                .gap(px(4.0))
                .items_center()
                .py(px(3.0))
                .pr(px(6.0))
                .pl(px((10 + depth * 12) as f32))
                .rounded(px(3.0))
                .cursor_pointer()
                .hover(move |this| this.bg(hover_bg))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.toggle_commit_files_dir(dir_clone.clone(), cx);
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
        Row::File { idx, depth } => {
            let f = &files[*idx];
            let code = code_to_letter(f.staged);
            let code_color = code_letter_color(code, muted_fg);
            let label = match (&f.old_path, &f.path) {
                (Some(old), new) if old != new => {
                    let old_base = old.rsplit('/').next().unwrap_or(old.as_str());
                    let new_base = new.rsplit('/').next().unwrap_or(new.as_str());
                    format!("{old_base} → {new_base}")
                }
                _ => f
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or(f.path.as_str())
                    .to_string(),
            };
            let is_selected = selected.as_deref() == Some(f.path.as_str());
            let path_for_click = f.path.clone();
            let commit_for_click = commit_id.to_string();
            let id = SharedString::from(format!("vcs-cd-file-{idx_in_rows}-{}", f.path));
            let mut row = h_flex()
                .id(id)
                .gap(px(8.0))
                .items_center()
                .py(px(3.0))
                .pr(px(6.0))
                .pl(px((10 + depth * 12 + 12) as f32))
                .rounded(px(3.0))
                .cursor_pointer()
                .hover(move |this| this.bg(hover_bg))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.select_commit_file(path_for_click.clone(), commit_for_click.clone(), cx);
                }))
                .child(
                    div()
                        .flex_none()
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
                        .text_xs()
                        .text_color(fg)
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(label),
                );
            if is_selected {
                row = row.bg(sel_bg);
            }
            row.into_any_element()
        }
    }
}
