//! Diff 渲染面板：Split 模式（IDEA 风格 sticky gutter + 双栏独立横滚）
//!
//! 架构（参考 zed/crates/editor/src/split.rs 的 lhs/rhs 双 Editor 思路）：
//! 每栏拆成 **gutter** 与 **content** 两个 uniform_list，共 4 个 list：
//!
//! ```text
//! ┌────────┬────────────────┬────┬────────┬────────────────┐
//! │ L gut  │ L content      │1px │ R gut  │ R content      │
//! │ (钉死) │ (横滚 h_left)   │div │ (钉死) │ (横滚 h_right)  │
//! └────────┴────────────────┴────┴────────┴────────────────┘
//! ```
//!
//! - 4 个 list 共享同一个 `UniformListScrollHandle` → 行级垂直同步
//! - 两栏的 content 区各有独立 `ScrollHandle` → 长行各自横滚不互相牵连
//! - gutter（checkbox + marker + 行号 [+ blame chip on R]）**在 overflow_x_scroll 之外**
//!   → 横滚时 gutter 永远可见
//!
//! cell 级渲染拆到 [`super::diff_split_cells`] 控制单文件 ≤ 600 行。
//!
//! 注意点：
//! - gpui-component 的 `h_flex()` 默认 `items_center`，必须显式 `.items_stretch()`，否则
//!   子栏被压成内容高（变白板）
//! - render 期间 entity 已被框架 mut 借用，禁止函数内 `cx.entity().read(cx)`，
//!   `has_blame` / `expanded_spacers` 由调用方从 `&self` 读出后传入

use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement, ScrollHandle,
    SharedString, Styled, UniformListScrollHandle, div, prelude::*, px, uniform_list,
};
use gpui_component::{ActiveTheme, h_flex};
use ramag_domain::entities::{DiffLineKind, FileDiff};

use super::diff_keys::{SplitKey, build_split_keys};
use super::diff_panel::{
    CHECKBOX_W, CONTENT_PAD, LINE_NO_W, MONO_CHAR_W, RestrictScrollExt as _, SPLIT_MARKER_W,
    render_diff_empty, render_file_diff,
};
use super::diff_split_cells::{
    BLAME_CHIP_W, render_content_cell, render_content_header, render_content_spacer,
    render_gutter_cell, render_gutter_header, render_gutter_spacer,
};
use super::vcs_view::VcsView;

/// gutter 固定宽：checkbox(18) + marker(10) + lineno(40) = 68px
const SPLIT_GUTTER_W: f32 = CHECKBOX_W + SPLIT_MARKER_W + LINE_NO_W;

/// 计算左右两栏各自最长行的字符数（旧 / 新分别算）
fn split_max_chars(diff: &FileDiff) -> (usize, usize) {
    let mut max_old = 0usize;
    let mut max_new = 0usize;
    for h in &diff.hunks {
        for l in &h.lines {
            let n = l.text.chars().count();
            match l.kind {
                DiffLineKind::Delete => max_old = max_old.max(n),
                DiffLineKind::Add => max_new = max_new.max(n),
                DiffLineKind::Context => {
                    max_old = max_old.max(n);
                    max_new = max_new.max(n);
                }
            }
        }
    }
    (max_old, max_new)
}

/// 判断 diff 是否是「单边」：纯新增（无 Delete / Context）或纯删除（无 Add / Context）
///
/// split 模式下另一栏永远空白，这种文件统一退化为 unified 单栏渲染避免视觉浪费
fn is_one_sided(diff: &FileDiff) -> bool {
    let mut has_old = false;
    let mut has_new = false;
    for h in &diff.hunks {
        for l in &h.lines {
            match l.kind {
                DiffLineKind::Delete => has_old = true,
                DiffLineKind::Add => has_new = true,
                DiffLineKind::Context => {
                    has_old = true;
                    has_new = true;
                }
            }
            if has_old && has_new {
                return false;
            }
        }
    }
    !(has_old && has_new)
}

/// 渲染整个文件的 diff（Split 模式，IDEA 风格双栏独立横滚 + sticky gutter）
#[allow(clippy::too_many_arguments)]
pub fn render_file_diff_split(
    diff: &FileDiff,
    _selected: &HashSet<(usize, usize)>,
    enable_selection: bool,
    changes_only: bool,
    mono: SharedString,
    _fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    _muted_bg: gpui::Hsla,
    scroll: &UniformListScrollHandle,
    h_scroll_left: &ScrollHandle,
    h_scroll_right: &ScrollHandle,
    has_blame: bool,
    expanded_spacers: &std::collections::HashSet<(usize, usize)>,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    if let Some(empty) = render_diff_empty(diff, muted_fg) {
        return empty;
    }
    if is_one_sided(diff) {
        return render_file_diff(
            diff,
            _selected,
            enable_selection,
            changes_only,
            mono,
            _fg,
            muted_fg,
            _muted_bg,
            scroll,
            h_scroll_left,
            cx,
        );
    }

    let diff_rc: Rc<FileDiff> = Rc::new(diff.clone());
    let keys: Rc<Vec<SplitKey>> =
        Rc::new(build_split_keys(&diff_rc, changes_only, expanded_spacers));
    let total = keys.len();

    let scroll_v = scroll.clone();
    let h_left = h_scroll_left.clone();
    let h_right = h_scroll_right.clone();

    let (max_old, max_new) = split_max_chars(&diff_rc);
    let left_content_w = (max_old as f32) * MONO_CHAR_W + CONTENT_PAD;
    let right_content_w = (max_new as f32) * MONO_CHAR_W + CONTENT_PAD;
    let right_gutter_w = if has_blame {
        SPLIT_GUTTER_W + BLAME_CHIP_W
    } else {
        SPLIT_GUTTER_W
    };

    let left_gutter_list = build_gutter_list(
        "L",
        true,
        false,
        total,
        diff_rc.clone(),
        keys.clone(),
        mono.clone(),
        enable_selection,
        scroll_v.clone(),
        cx,
    );
    let left_content_list = build_content_list(
        "L",
        true,
        total,
        diff_rc.clone(),
        keys.clone(),
        mono.clone(),
        enable_selection,
        left_content_w,
        scroll_v.clone(),
        cx,
    );
    let right_gutter_list = build_gutter_list(
        "R",
        false,
        has_blame,
        total,
        diff_rc.clone(),
        keys.clone(),
        mono.clone(),
        enable_selection,
        scroll_v.clone(),
        cx,
    );
    let right_content_list = build_content_list(
        "R",
        false,
        total,
        diff_rc,
        keys,
        mono,
        enable_selection,
        right_content_w,
        scroll_v,
        cx,
    );

    h_flex()
        .items_stretch()
        .size_full()
        .min_w_0()
        .min_h_0()
        .child(make_pane(
            left_gutter_list,
            left_content_list,
            SPLIT_GUTTER_W,
            left_content_w,
            &h_left,
            "L",
        ))
        .child(div().flex_none().w(px(1.0)).h_full().bg(muted_fg))
        .child(make_pane(
            right_gutter_list,
            right_content_list,
            right_gutter_w,
            right_content_w,
            &h_right,
            "R",
        ))
        .into_any_element()
}

/// 单栏布局：[gutter 固定 w][content overflow_x_scroll]
fn make_pane(
    gutter: gpui::UniformList,
    content: gpui::UniformList,
    gutter_w: f32,
    content_w: f32,
    h_handle: &ScrollHandle,
    side: &'static str,
) -> impl IntoElement {
    h_flex()
        .items_stretch()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .h_full()
        .child(div().flex_none().w(px(gutter_w)).h_full().child(gutter))
        .child(
            div()
                .id(SharedString::from(format!("vcs-diff-{side}-h-scroll")))
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_x_scroll()
                .restrict_scroll_to_axis()
                .track_scroll(h_handle)
                .child(
                    gpui_component::v_flex()
                        .min_w_full()
                        .w(px(content_w))
                        .h_full()
                        .child(content),
                ),
        )
}

/// 构建一栏的 gutter uniform_list（钉死区）
#[allow(clippy::too_many_arguments)]
fn build_gutter_list(
    side: &'static str,
    is_left: bool,
    has_blame: bool,
    total: usize,
    diff_rc: Rc<FileDiff>,
    keys: Rc<Vec<SplitKey>>,
    mono: SharedString,
    enable_selection: bool,
    scroll_v: UniformListScrollHandle,
    cx: &mut Context<VcsView>,
) -> gpui::UniformList {
    uniform_list(
        SharedString::from(format!("vcs-diff-{side}-gutter")),
        total,
        cx.processor(move |this, range: Range<usize>, _w, cx| {
            let theme = cx.theme();
            let muted_fg = theme.muted_foreground;
            let muted_bg = theme.muted;
            let accent = theme.accent;
            let selected = this.selected_diff_lines.clone();
            let blame_rc: Option<Rc<Vec<ramag_domain::entities::BlameLine>>> = if has_blame
                && this.showing_blame
                && !this.blame_lines.is_empty()
            {
                Some(Rc::new(this.blame_lines.clone()))
            } else {
                None
            };
            range
                .map(|i| match keys[i] {
                    SplitKey::Header { hunk_idx } => render_gutter_header(
                        side,
                        hunk_idx,
                        enable_selection,
                        is_left,
                        muted_bg,
                        cx,
                    ),
                    SplitKey::Pair {
                        hunk_idx,
                        left,
                        right,
                    } => {
                        let line_idx = if is_left { left } else { right };
                        let line = line_idx.map(|li| (li, &diff_rc.hunks[hunk_idx].lines[li]));
                        render_gutter_cell(
                            side,
                            line,
                            hunk_idx,
                            &selected,
                            enable_selection,
                            is_left,
                            has_blame,
                            blame_rc.as_ref(),
                            muted_fg,
                            accent,
                            mono.clone(),
                            cx,
                        )
                    }
                    SplitKey::Spacer { .. } => render_gutter_spacer(side, muted_bg),
                })
                .collect::<Vec<_>>()
        }),
    )
    .track_scroll(&scroll_v)
    .h_full()
    .min_h_0()
}

/// 构建一栏的 content uniform_list（横滚区，仅渲染代码文本）
#[allow(clippy::too_many_arguments)]
fn build_content_list(
    side: &'static str,
    is_left: bool,
    total: usize,
    diff_rc: Rc<FileDiff>,
    keys: Rc<Vec<SplitKey>>,
    mono: SharedString,
    enable_selection: bool,
    content_w: f32,
    scroll_v: UniformListScrollHandle,
    cx: &mut Context<VcsView>,
) -> gpui::UniformList {
    uniform_list(
        SharedString::from(format!("vcs-diff-{side}-content")),
        total,
        cx.processor(move |this, range: Range<usize>, _w, cx| {
            let theme = cx.theme();
            let fg = theme.foreground;
            let muted_fg = theme.muted_foreground;
            let muted_bg = theme.muted;
            let accent = theme.accent;
            let selected = this.selected_diff_lines.clone();
            range
                .map(|i| match keys[i] {
                    SplitKey::Header { hunk_idx } => render_content_header(
                        &diff_rc.hunks[hunk_idx],
                        mono.clone(),
                        muted_fg,
                        muted_bg,
                    ),
                    SplitKey::Pair {
                        hunk_idx,
                        left,
                        right,
                    } => {
                        let line_idx = if is_left { left } else { right };
                        let line = line_idx.map(|li| (li, &diff_rc.hunks[hunk_idx].lines[li]));
                        render_content_cell(
                            side,
                            line,
                            hunk_idx,
                            &selected,
                            enable_selection,
                            fg,
                            accent,
                            mono.clone(),
                            content_w,
                            cx,
                        )
                    }
                    SplitKey::Spacer {
                        hunk_idx,
                        run_start,
                        skipped,
                    } => render_content_spacer(side, hunk_idx, run_start, skipped, muted_fg, cx),
                })
                .collect::<Vec<_>>()
        }),
    )
    .track_scroll(&scroll_v)
    .w(px(content_w))
    .min_w_full()
    .restrict_scroll_to_axis()
    .h_full()
    .min_h_0()
}
