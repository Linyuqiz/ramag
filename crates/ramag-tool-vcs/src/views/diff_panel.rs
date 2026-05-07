//! Diff 面板：Unified（行号双列 + `+/-`）/ Split（左旧右新对齐）。`+/-` 行点击 = `toggle_diff_line`

use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement,
    ScrollHandle, SharedString, Styled, UniformListScrollHandle, div, prelude::*, px, uniform_list,
};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable as _, h_flex};
use ramag_domain::entities::{DiffLine, DiffLineKind, FileDiff};

use super::vcs_view::VcsView;

/// 单行高度（uniform_list 要求等高，hunk header 也压缩到这个高度）
pub(super) const DIFF_ROW_H: f32 = 20.0;
/// 等宽字体单字符估算宽度（mono 13px size 下约 7.5px/字，与 pf_content 同款）
pub(super) const MONO_CHAR_W: f32 = 7.5;
/// 行号列固定宽度（含左右 padding）
pub(super) const LINE_NO_W: f32 = 40.0;
/// checkbox 单元格固定宽度
pub(super) const CHECKBOX_W: f32 = 18.0;
/// Unified marker 列宽（+/-）
const UNIFIED_MARKER_W: f32 = 14.0;
/// Split marker 列宽（+/-，比 unified 略窄）
pub(super) const SPLIT_MARKER_W: f32 = 10.0;
/// 行内容左右 padding（×2）
pub(super) const CONTENT_PAD: f32 = 8.0;

use super::diff_keys::{UnifiedKey, build_unified_keys};

/// 关闭 GPUI 单轴 scroll 的"另一方向劫持"行为（与 pf_content 同款 trick）
pub(super) trait RestrictScrollExt: Styled + Sized {
    fn restrict_scroll_to_axis(mut self) -> Self {
        self.style().restrict_scroll_to_axis = Some(true);
        self
    }
}
impl<T: Styled> RestrictScrollExt for T {}

/// 计算 diff 中最长行字符数（unified / split 公用，决定行内容固定宽度）
pub(super) fn max_line_chars(diff: &FileDiff) -> usize {
    let mut max = 0usize;
    for h in &diff.hunks {
        for l in &h.lines {
            let n = l.text.chars().count();
            if n > max {
                max = n;
            }
        }
    }
    max
}

/// Unified diff。固定 list w + 外层 overflow_x_scroll 共享 ScrollHandle，restrict_scroll_to_axis 防 wheel 错位
#[allow(clippy::too_many_arguments)]
pub fn render_file_diff(
    diff: &FileDiff,
    _selected: &HashSet<(usize, usize)>,
    enable_selection: bool,
    changes_only: bool,
    mono: SharedString,
    _fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    _muted_bg: gpui::Hsla,
    scroll: &UniformListScrollHandle,
    h_scroll: &ScrollHandle,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    if let Some(empty) = render_diff_empty(diff, muted_fg) {
        return empty;
    }
    let diff_rc: Rc<FileDiff> = Rc::new(diff.clone());
    let keys: Rc<Vec<UnifiedKey>> = Rc::new(build_unified_keys(&diff_rc, changes_only));
    let total = keys.len();
    let scroll = scroll.clone();
    let h_scroll = h_scroll.clone();

    let max_chars = max_line_chars(&diff_rc);
    let content_w = (max_chars as f32) * MONO_CHAR_W + CONTENT_PAD;
    let total_w = CHECKBOX_W + LINE_NO_W * 2.0 + UNIFIED_MARKER_W + content_w;

    let body = uniform_list(
        "vcs-diff-unified",
        total,
        cx.processor({
            let diff_rc = diff_rc.clone();
            let keys = keys.clone();
            let mono = mono.clone();
            move |this, range: Range<usize>, _w, cx| {
                let theme = cx.theme();
                let fg = theme.foreground;
                let muted_fg = theme.muted_foreground;
                let muted_bg = theme.muted;
                let accent = theme.accent;
                let selected = this.selected_diff_lines.clone();
                range
                    .map(|i| match keys[i] {
                        UnifiedKey::Header { hunk_idx } => render_hunk_header_unified(
                            &diff_rc.hunks[hunk_idx],
                            hunk_idx,
                            enable_selection,
                            mono.clone(),
                            muted_fg,
                            muted_bg,
                            cx,
                        )
                        .into_any_element(),
                        UnifiedKey::Line { hunk_idx, line_idx } => {
                            let line = &diff_rc.hunks[hunk_idx].lines[line_idx];
                            let is_sel = selected.contains(&(hunk_idx, line_idx));
                            render_diff_line(
                                line,
                                hunk_idx,
                                line_idx,
                                is_sel,
                                enable_selection,
                                mono.clone(),
                                fg,
                                muted_fg,
                                accent,
                                content_w,
                                cx,
                            )
                            .into_any_element()
                        }
                    })
                    .collect::<Vec<_>>()
            }
        }),
    )
    .track_scroll(&scroll)
    .w(px(total_w))
    .min_w_full()
    .restrict_scroll_to_axis()
    .flex_1();

    div()
        .id("vcs-diff-unified-h-scroll")
        .size_full()
        .min_w_0()
        .min_h_0()
        .overflow_x_scroll()
        .restrict_scroll_to_axis()
        .track_scroll(&h_scroll)
        .child(
            gpui_component::v_flex()
                .min_w_full()
                .w(px(total_w))
                .h_full()
                .child(body),
        )
        .into_any_element()
}

/// hunk header unified：整行宽，enable_discard 时显示回滚按钮
pub(super) fn render_hunk_header_unified(
    hunk: &ramag_domain::entities::Hunk,
    hunk_idx: usize,
    enable_discard: bool,
    mono: SharedString,
    muted_fg: gpui::Hsla,
    muted_bg: gpui::Hsla,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    render_hunk_header_common(hunk, hunk_idx, enable_discard, mono, muted_fg, muted_bg, cx)
}

/// hunk header 通用渲染：左 hunk text + 右可选回滚按钮
pub(super) fn render_hunk_header_common(
    hunk: &ramag_domain::entities::Hunk,
    hunk_idx: usize,
    enable_discard: bool,
    mono: SharedString,
    muted_fg: gpui::Hsla,
    muted_bg: gpui::Hsla,
    cx: &mut Context<VcsView>,
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
    let row = h_flex()
        .w_full()
        .h(px(DIFF_ROW_H))
        .flex_none()
        .px(px(8.0))
        .bg(muted_bg)
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(muted_fg)
        .font_family(mono)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_nowrap()
                .overflow_hidden()
                .child(header_text),
        );
    // unified 回滚按钮在 split 中间分隔条，这里不渲染
    let _ = enable_discard;
    let _ = hunk_idx;
    let _ = cx;
    row.into_any_element()
}

/// 二进制 / 无差异时给出占位元素，否则 None
pub(super) fn render_diff_empty(diff: &FileDiff, muted_fg: gpui::Hsla) -> Option<AnyElement> {
    if diff.binary {
        return Some(
            div()
                .px(px(12.0))
                .py(px(20.0))
                .text_sm()
                .text_color(muted_fg)
                .child("（二进制文件，不渲染内容）")
                .into_any_element(),
        );
    }
    if diff.hunks.is_empty() {
        return Some(
            div()
                .px(px(12.0))
                .py(px(20.0))
                .text_sm()
                .text_color(muted_fg)
                .child("（无差异）")
                .into_any_element(),
        );
    }
    None
}

/// Unified 单行 diff：[checkbox][old_no][new_no][marker][content (flex_1 + nowrap)]
///
/// 整个 list 外层 overflow_x_scroll 包住 → 行不再有自己的横滚 cell
#[allow(clippy::too_many_arguments)]
fn render_diff_line(
    line: &DiffLine,
    hunk_idx: usize,
    line_idx: usize,
    selected: bool,
    enable_selection: bool,
    mono: SharedString,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    accent: gpui::Hsla,
    content_w: f32,
    cx: &mut Context<VcsView>,
) -> impl IntoElement {
    let (bg, marker, marker_color) = line_palette(line.kind);
    let old_label = line.old_lineno.map(|n| n.to_string()).unwrap_or_default();
    let new_label = line.new_lineno.map(|n| n.to_string()).unwrap_or_default();
    let toggleable = enable_selection && line.kind != DiffLineKind::Context;

    let mut row_bg = bg;
    if selected {
        let mut sel = accent;
        sel.a = 0.22;
        row_bg = Some(sel);
    }
    let row_id = SharedString::from(format!("vcs-diff-line-{hunk_idx}-{line_idx}"));
    let old_id = SharedString::from(format!("vcs-diff-old-{hunk_idx}-{line_idx}"));
    let new_id = SharedString::from(format!("vcs-diff-new-{hunk_idx}-{line_idx}"));
    let _ = (old_label, new_label);
    let mut row = h_flex()
        .id(row_id)
        .w_full()
        .h(px(DIFF_ROW_H))
        .flex_none()
        .gap(px(0.0))
        .font_family(mono.clone())
        .text_xs()
        .child(checkbox_cell(toggleable, selected, accent, muted_fg))
        .child(line_no_cell_clickable(
            line.old_lineno,
            true,
            old_id,
            muted_fg,
            cx,
        ))
        .child(line_no_cell_clickable(
            line.new_lineno,
            false,
            new_id,
            muted_fg,
            cx,
        ))
        .child(
            div()
                .flex_none()
                .w(px(UNIFIED_MARKER_W))
                .text_color(marker_color)
                .child(marker),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(content_w))
                .px(px(4.0))
                .text_color(fg)
                .font_family(mono)
                .whitespace_nowrap()
                .child(line.text.clone()),
        );

    if toggleable {
        row = row
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.toggle_diff_line(hunk_idx, line_idx, cx);
            }));
    }
    if let Some(c) = row_bg {
        row = row.bg(c);
    }
    row
}

/// 行级选中复选框单元格（不可勾选时仅占位保持对齐）
pub(super) fn checkbox_cell(
    enabled: bool,
    selected: bool,
    accent: gpui::Hsla,
    muted_fg: gpui::Hsla,
) -> impl IntoElement {
    let icon: Option<AnyElement> = if !enabled {
        None
    } else if selected {
        Some(
            Icon::new(IconName::Check)
                .xsmall()
                .text_color(accent)
                .into_any_element(),
        )
    } else {
        Some(
            Icon::new(IconName::Plus)
                .xsmall()
                .text_color({
                    let mut c = muted_fg;
                    c.a = 0.5;
                    c
                })
                .into_any_element(),
        )
    };
    let mut cell = div()
        .flex_none()
        .w(px(18.0))
        .flex()
        .items_center()
        .justify_center();
    if let Some(i) = icon {
        cell = cell.child(i);
    }
    cell
}

/// 公共行号单元格（40px 宽 / 右对齐风）
pub(super) fn line_no_cell(label: String, muted_fg: gpui::Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(40.0))
        .px(px(4.0))
        .text_color(muted_fg)
        .child(label)
}

/// 可点击的行号单元格：点击 → 顶部 banner 显示该行的 inline blame
///
/// `line_no=None` 时退化为静态单元格（空配对侧无 line_no）；is_old 用于区分 blame 取 old/new 行号
pub(super) fn line_no_cell_clickable(
    line_no: Option<u32>,
    is_old: bool,
    cell_id: SharedString,
    muted_fg: gpui::Hsla,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let label = line_no.map(|n| n.to_string()).unwrap_or_default();
    let mut cell = div()
        .id(cell_id)
        .flex_none()
        .w(px(40.0))
        .px(px(4.0))
        .text_color(muted_fg)
        .child(label);
    if let Some(n) = line_no {
        cell = cell
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.show_inline_blame(n, is_old, cx);
            }));
    }
    cell.into_any_element()
}

/// 行类型 → (背景色, 标记字符, 标记色)
pub(super) fn line_palette(kind: DiffLineKind) -> (Option<gpui::Hsla>, &'static str, gpui::Hsla) {
    match kind {
        DiffLineKind::Context => (None, " ", gpui::hsla(0.0, 0.0, 0.5, 1.0)),
        DiffLineKind::Add => (
            Some(gpui::hsla(140.0 / 360.0, 0.55, 0.85, 0.30)),
            "+",
            gpui::hsla(140.0 / 360.0, 0.55, 0.40, 1.0),
        ),
        DiffLineKind::Delete => (
            Some(gpui::hsla(0.0, 0.65, 0.85, 0.30)),
            "-",
            gpui::hsla(0.0, 0.65, 0.50, 1.0),
        ),
    }
}
