//! Split diff 单元格渲染（IDEA 风格 4-list 架构的 cell helper）
//!
//! 拆分自 `diff_panel_split.rs`，控制单文件 ≤ 600 行。
//!
//! 提供两套 row 渲染：
//! - `gutter_*`：钉死区（不进 overflow_x_scroll）—— checkbox + marker + lineno [+ blame]
//! - `content_*`：横滚区 —— 仅代码文本 / @@ 头 / 跳过 spacer 文本
//!
//! 同一个 SplitKey 由 gutter 与 content 各渲染一份，所属 list 共享垂直 scroll → 行级对齐

use std::collections::HashSet;
use std::rc::Rc;

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement,
    SharedString, Styled, div, prelude::*, px,
};
use gpui_component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};
use ramag_domain::entities::{DiffLine, DiffLineKind};

use super::diff_panel::{
    DIFF_ROW_H, SPLIT_MARKER_W, checkbox_cell, line_no_cell, line_no_cell_clickable, line_palette,
};
use super::vcs_view::VcsView;

/// blame author chip 宽（仅右栏 gutter 启用 blame 时追加）
pub(super) const BLAME_CHIP_W: f32 = 96.0;

/// gutter 单元格：左栏 `[checkbox][marker][lineno]`；右栏 `[lineno][marker][checkbox][blame_chip?]`
#[allow(clippy::too_many_arguments)]
pub(super) fn render_gutter_cell(
    side: &'static str,
    line: Option<(usize, &DiffLine)>,
    hunk_idx: usize,
    selected: &HashSet<(usize, usize)>,
    enable_selection: bool,
    is_left: bool,
    has_blame: bool,
    blame: Option<&Rc<Vec<ramag_domain::entities::BlameLine>>>,
    muted_fg: gpui::Hsla,
    accent: gpui::Hsla,
    mono: SharedString,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let blame_slot_w = if has_blame { BLAME_CHIP_W } else { 0.0 };
    let Some((line_idx, line)) = line else {
        // 空行（对侧专属）：保持高度对齐，渲染同宽度但无内容
        let mut empty = h_flex().h(px(DIFF_ROW_H));
        if is_left {
            empty = empty
                .child(checkbox_cell(false, false, accent, muted_fg))
                .child(div().flex_none().w(px(SPLIT_MARKER_W)))
                .child(line_no_cell(String::new(), muted_fg));
        } else {
            empty = empty
                .child(line_no_cell(String::new(), muted_fg))
                .child(div().flex_none().w(px(SPLIT_MARKER_W)))
                .child(checkbox_cell(false, false, accent, muted_fg))
                .child(div().flex_none().w(px(blame_slot_w)));
        }
        return empty.into_any_element();
    };

    let (_bg, _, marker_color) = line_palette(line.kind);
    let toggleable = enable_selection && line.kind != DiffLineKind::Context;
    let is_sel = selected.contains(&(hunk_idx, line_idx));
    let row_id = SharedString::from(format!("vcs-diff-gut-{side}-{hunk_idx}-{line_idx}"));
    let lineno_id = SharedString::from(format!("vcs-diff-ln-{side}-{hunk_idx}-{line_idx}"));
    let lineno_value = if is_left {
        line.old_lineno
    } else {
        line.new_lineno
    };
    let lineno_div = line_no_cell_clickable(lineno_value, is_left, lineno_id, muted_fg, cx);

    let marker_div = div()
        .flex_none()
        .w(px(SPLIT_MARKER_W))
        .text_color(marker_color)
        .font_family(mono.clone())
        .child(match line.kind {
            DiffLineKind::Add => "+",
            DiffLineKind::Delete => "-",
            DiffLineKind::Context => " ",
        });

    let blame_chip: AnyElement = if has_blame {
        match line.new_lineno.and_then(|ln| {
            blame
                .and_then(|bs| bs.iter().find(|b| b.line_no == ln))
                .map(|b| b.author.chars().take(14).collect::<String>())
        }) {
            Some(author) => div()
                .flex_none()
                .w(px(BLAME_CHIP_W))
                .px(px(4.0))
                .text_xs()
                .text_color(muted_fg)
                .font_family(mono.clone())
                .overflow_hidden()
                .text_ellipsis()
                .child(author)
                .into_any_element(),
            None => div().flex_none().w(px(BLAME_CHIP_W)).into_any_element(),
        }
    } else {
        div().into_any_element()
    };

    let mut row = h_flex().id(row_id).h(px(DIFF_ROW_H));
    if is_left {
        row = row
            .child(checkbox_cell(toggleable, is_sel, accent, muted_fg))
            .child(marker_div)
            .child(lineno_div);
    } else {
        row = row
            .child(lineno_div)
            .child(marker_div)
            .child(checkbox_cell(toggleable, is_sel, accent, muted_fg))
            .child(blame_chip);
    }
    if toggleable {
        row = row
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.toggle_diff_line(hunk_idx, line_idx, cx);
            }));
    }
    if is_sel {
        let mut sel = accent;
        sel.a = 0.22;
        row = row.bg(sel);
    }
    row.into_any_element()
}

/// content 单元格：仅渲染代码文本，宽度由外层 list `w(content_w)` 撑开
#[allow(clippy::too_many_arguments)]
pub(super) fn render_content_cell(
    side: &'static str,
    line: Option<(usize, &DiffLine)>,
    hunk_idx: usize,
    selected: &HashSet<(usize, usize)>,
    enable_selection: bool,
    fg: gpui::Hsla,
    accent: gpui::Hsla,
    mono: SharedString,
    content_w: f32,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let Some((line_idx, line)) = line else {
        return h_flex()
            .h(px(DIFF_ROW_H))
            .min_w(px(content_w))
            .into_any_element();
    };
    let (bg, _, _) = line_palette(line.kind);
    let toggleable = enable_selection && line.kind != DiffLineKind::Context;
    let is_sel = selected.contains(&(hunk_idx, line_idx));
    let row_id = SharedString::from(format!("vcs-diff-cnt-{side}-{hunk_idx}-{line_idx}"));

    let text_div = div()
        .flex_1()
        .min_w(px(content_w))
        .px(px(4.0))
        .text_color(fg)
        .font_family(mono)
        .whitespace_nowrap()
        .child(line.text.clone());

    let mut row = h_flex()
        .id(row_id)
        .h(px(DIFF_ROW_H))
        .min_w(px(content_w))
        .child(text_div);
    if toggleable {
        row = row
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.toggle_diff_line(hunk_idx, line_idx, cx);
            }));
    }
    if is_sel {
        let mut sel = accent;
        sel.a = 0.22;
        row = row.bg(sel);
    } else if let Some(c) = bg {
        row = row.bg(c);
    }
    row.into_any_element()
}

/// gutter hunk header：背景色 + 仅左栏带 ↶ 撤销按钮
pub(super) fn render_gutter_header(
    side: &'static str,
    hunk_idx: usize,
    enable_discard: bool,
    is_left: bool,
    muted_bg: gpui::Hsla,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let mut row = h_flex()
        .w_full()
        .h(px(DIFF_ROW_H))
        .flex_none()
        .bg(muted_bg)
        .items_center()
        .justify_end()
        .px(px(2.0));
    if is_left && enable_discard {
        let id = SharedString::from(format!("vcs-hunk-discard-{side}-{hunk_idx}"));
        row = row.child(
            Button::new(id)
                .ghost()
                .xsmall()
                .icon(gpui_component::IconName::Undo)
                .tooltip("回滚此 hunk（Staged→unstage / Unstaged→discard）")
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.discard_hunk(hunk_idx, cx);
                })),
        );
    }
    row.into_any_element()
}

/// content hunk header：渲染 @@ 文本（hunk 位置信息），左右两栏各一份相同文本保持视觉对齐
pub(super) fn render_content_header(
    hunk: &ramag_domain::entities::Hunk,
    mono: SharedString,
    muted_fg: gpui::Hsla,
    muted_bg: gpui::Hsla,
) -> AnyElement {
    let header_text = format!(
        "@@ -{},{} +{},{} @@{}",
        hunk.old_start,
        hunk.old_lines,
        hunk.new_start,
        hunk.new_lines,
        match &hunk.heading {
            Some(h) => format!(" {h}"),
            None => String::new(),
        }
    );
    h_flex()
        .w_full()
        .h(px(DIFF_ROW_H))
        .flex_none()
        .bg(muted_bg)
        .px(px(8.0))
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(muted_fg)
        .font_family(mono)
        .whitespace_nowrap()
        .overflow_hidden()
        .child(SharedString::from(header_text))
        .into_any_element()
}

/// gutter spacer：仅背景色（点击交互在 content 列承担）
pub(super) fn render_gutter_spacer(_side: &'static str, muted_bg: gpui::Hsla) -> AnyElement {
    div()
        .h(px(DIFF_ROW_H))
        .w_full()
        .bg(muted_bg)
        .into_any_element()
}

/// content spacer：「跳过 X 行（点击展开）」整行可点击触发 expanded_diff_spacers 写入
pub(super) fn render_content_spacer(
    side: &'static str,
    hunk_idx: usize,
    run_start: usize,
    skipped: usize,
    muted_fg: gpui::Hsla,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let row_id = SharedString::from(format!(
        "vcs-diff-spacer-{side}-{hunk_idx}-{run_start}"
    ));
    h_flex()
        .id(row_id)
        .w_full()
        .h(px(DIFF_ROW_H))
        .flex_none()
        .items_center()
        .justify_center()
        .text_xs()
        .text_color(muted_fg)
        .cursor_pointer()
        .child(format!(
            "───── 跳过 {skipped} 行未变更（点击展开） ─────"
        ))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.expanded_diff_spacers.insert((hunk_idx, run_start));
            cx.notify();
        }))
        .into_any_element()
}
