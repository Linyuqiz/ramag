//! Split diff：左 gutter+content、中间列（回滚/blame）、右 gutter+content 共 5 个 uniform_list。
//! 5 个 list 共享 `UniformListScrollHandle` 行级 Y 同步；content 各自独立 X 滚；gutter / 中间列在 overflow_x_scroll 之外保持可见。
//! `h_flex` 默认 items_center，必须显式 `.items_stretch()` 否则子栏会被压成内容高

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement,
    ScrollHandle, SharedString, Styled, UniformListScrollHandle, div, prelude::*, px, uniform_list,
};
use gpui_component::{
    ActiveTheme, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};
use ramag_domain::entities::{DiffLineKind, FileDiff};

use super::diff_keys::{SplitKey, build_split_keys};
use super::diff_panel::{
    CONTENT_PAD, DIFF_ROW_H, LINE_NO_W, MONO_CHAR_W, RestrictScrollExt as _, SPLIT_MARKER_W,
    render_diff_empty, render_file_diff,
};
use super::diff_split_cells::{
    render_content_cell, render_content_header, render_content_spacer, render_gutter_cell,
    render_gutter_header, render_gutter_spacer,
};
use super::vcs_view::VcsView;

/// gutter 固定宽：marker(10) + lineno(40) = 50px
const SPLIT_GUTTER_W: f32 = SPLIT_MARKER_W + LINE_NO_W;

/// 计算左右两栏各自最长行的字符数（旧 / 新分别算）
fn split_max_chars(diff: &FileDiff) -> (usize, usize) {
    let mut max_old = 0usize;
    let mut max_new = 0usize;
    for h in &diff.hunks {
        for l in &h.lines {
            let n = super::syntax::display_cols(&l.text);
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

/// 渲染整个文件的 diff（Split 模式，IDEA 风格双栏独立横滚 + sticky gutter + 中间列）
#[allow(clippy::too_many_arguments)]
pub fn render_file_diff_split(
    diff: &FileDiff,
    enable_discard: bool,
    changes_only: bool,
    // false=不折叠长 Context（FullFile 模式展示所有内容）
    collapse: bool,
    // 语法高亮语言（None=纯文本，由调用方按文件扩展名算）
    lang: Option<SharedString>,
    mono: SharedString,
    _fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    muted_bg: gpui::Hsla,
    scroll: &UniformListScrollHandle,
    // 左右两栏共享同一横滚 handle，两栏一起横滚（IDEA 风格，避免错位无法对比）
    h_scroll: &ScrollHandle,
    has_blame: bool,
    expanded_spacers: &HashSet<(usize, usize)>,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    if let Some(empty) = render_diff_empty(diff, muted_fg) {
        return empty;
    }
    if is_one_sided(diff) {
        return render_file_diff(
            diff,
            changes_only,
            lang,
            mono,
            _fg,
            muted_fg,
            muted_bg,
            scroll,
            h_scroll,
            cx,
        );
    }

    let diff_rc: Rc<FileDiff> = Rc::new(diff.clone());
    let keys: Rc<Vec<SplitKey>> = Rc::new(build_split_keys(
        &diff_rc,
        changes_only,
        collapse,
        expanded_spacers,
    ));
    let total = keys.len();

    // 每个 hunk 的「中点行」→ hunk_idx：回滚按钮放中点行（仿 VSCode 居中），而非 header 第一行
    let button_rows: Rc<HashMap<usize, usize>> = {
        let mut m: HashMap<usize, usize> = HashMap::new();
        let mut i = 0;
        while i < keys.len() {
            if let SplitKey::Header { hunk_idx } = keys[i] {
                let mut j = i + 1;
                while j < keys.len() && !matches!(keys[j], SplitKey::Header { .. }) {
                    j += 1;
                }
                let span = j - i;
                // 中点偏向内容行（header 之后），span=1 的退化 hunk 落回 header
                let mid = if span > 1 { i + span / 2 } else { i };
                m.insert(mid, hunk_idx);
                i = j;
            } else {
                i += 1;
            }
        }
        Rc::new(m)
    };

    let scroll_v = scroll.clone();
    let h_shared = h_scroll.clone();

    let (max_old, max_new) = split_max_chars(&diff_rc);
    // 左右共用同一内容宽度（取较长侧）：共享横滚 handle 时两栏滚动范围才一致，都能滚到行尾
    let content_w = (max_old.max(max_new) as f32) * MONO_CHAR_W + CONTENT_PAD;
    // 中间列：仅回滚按钮时窄（28），需展示 blame author 时宽（96）
    let middle_w = if has_blame { 140.0 } else { 28.0 };

    let left_gutter_list = build_gutter_list(
        "L",
        true,
        total,
        diff_rc.clone(),
        keys.clone(),
        mono.clone(),
        scroll_v.clone(),
        cx,
    );
    let left_content_list = build_content_list(
        "L",
        true,
        total,
        diff_rc.clone(),
        keys.clone(),
        lang.clone(),
        mono.clone(),
        content_w,
        scroll_v.clone(),
        cx,
    );
    let middle_list = build_middle_list(
        total,
        diff_rc.clone(),
        keys.clone(),
        button_rows.clone(),
        enable_discard,
        has_blame,
        middle_w,
        scroll_v.clone(),
        cx,
    );
    let right_gutter_list = build_gutter_list(
        "R",
        false,
        total,
        diff_rc.clone(),
        keys.clone(),
        mono.clone(),
        scroll_v.clone(),
        cx,
    );
    let right_content_list = build_content_list(
        "R", false, total, diff_rc, keys, lang, mono, content_w, scroll_v, cx,
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
            content_w,
            &h_shared,
            "L",
        ))
        .child(div().flex_none().w(px(1.0)).h_full().bg(muted_fg))
        .child(
            div()
                .flex_none()
                .w(px(middle_w))
                .h_full()
                .child(middle_list),
        )
        .child(div().flex_none().w(px(1.0)).h_full().bg(muted_fg))
        .child(make_pane(
            right_gutter_list,
            right_content_list,
            SPLIT_GUTTER_W,
            content_w,
            &h_shared,
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
    total: usize,
    diff_rc: Rc<FileDiff>,
    keys: Rc<Vec<SplitKey>>,
    mono: SharedString,
    scroll_v: UniformListScrollHandle,
    cx: &mut Context<VcsView>,
) -> gpui::UniformList {
    uniform_list(
        SharedString::from(format!("vcs-diff-{side}-gutter")),
        total,
        cx.processor(move |_this, range: Range<usize>, _w, cx| {
            let theme = cx.theme();
            let muted_fg = theme.muted_foreground;
            let muted_bg = theme.muted;
            range
                .map(|i| match keys[i] {
                    SplitKey::Header { .. } => render_gutter_header(muted_bg),
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
                            is_left,
                            muted_fg,
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
    lang: Option<SharedString>,
    mono: SharedString,
    content_w: f32,
    scroll_v: UniformListScrollHandle,
    cx: &mut Context<VcsView>,
) -> gpui::UniformList {
    uniform_list(
        SharedString::from(format!("vcs-diff-{side}-content")),
        total,
        cx.processor(move |_this, range: Range<usize>, _w, cx| {
            let theme = cx.theme();
            let fg = theme.foreground;
            let muted_fg = theme.muted_foreground;
            let muted_bg = theme.muted;
            let lang_ref = lang.as_deref();
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
                            lang_ref,
                            fg,
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

/// 构建中间列 uniform_list（与左右栏共享 scroll_v 做垂直同步）
///
/// Header 行承载「回滚此 hunk」按钮（enable_discard 时）；Pair 行展示该行 blame author（has_blame 时）
#[allow(clippy::too_many_arguments)]
fn build_middle_list(
    total: usize,
    diff_rc: Rc<FileDiff>,
    keys: Rc<Vec<SplitKey>>,
    button_rows: Rc<HashMap<usize, usize>>,
    enable_discard: bool,
    has_blame: bool,
    middle_w: f32,
    scroll_v: UniformListScrollHandle,
    cx: &mut Context<VcsView>,
) -> gpui::UniformList {
    uniform_list(
        "vcs-diff-middle",
        total,
        cx.processor(move |this, range: Range<usize>, _w, cx| {
            let theme = cx.theme();
            let muted_fg = theme.muted_foreground;
            let muted_bg = theme.muted;
            // blame 仅在开启 blame 且已加载时取数据，按 new_lineno 匹配
            let blame_rc: Option<Rc<Vec<ramag_domain::entities::BlameLine>>> =
                if has_blame && this.showing_blame && !this.blame_lines.is_empty() {
                    Some(Rc::new(this.blame_lines.clone()))
                } else {
                    None
                };
            range
                .map(|i| {
                    // hunk 中点行 + 可回滚：渲染居中回滚按钮（替换该行 blame，仿 VSCode）
                    if enable_discard && let Some(&hunk_idx) = button_rows.get(&i) {
                        return render_middle_revert(hunk_idx, cx);
                    }
                    match keys[i] {
                        SplitKey::Header { .. } => div()
                            .w_full()
                            .h(px(DIFF_ROW_H))
                            .bg(muted_bg)
                            .into_any_element(),
                        SplitKey::Pair {
                            hunk_idx,
                            left,
                            right,
                        } => {
                            // 左列=旧侧行作者、右列=新侧行作者（都按 new_lineno 查当前文件 blame）
                            let author_of = |li: Option<usize>| {
                                li.and_then(|i| diff_rc.hunks[hunk_idx].lines[i].new_lineno)
                                    .and_then(|ln| {
                                        blame_rc.as_ref().and_then(|bs| {
                                            bs.iter().find(|b| b.line_no == ln).map(|b| {
                                                b.author.chars().take(10).collect::<String>()
                                            })
                                        })
                                    })
                            };
                            render_middle_cell(author_of(left), author_of(right), muted_fg)
                        }
                        SplitKey::Spacer { .. } => div()
                            .h(px(DIFF_ROW_H))
                            .w_full()
                            .bg(muted_bg)
                            .into_any_element(),
                    }
                })
                .collect::<Vec<_>>()
        }),
    )
    .track_scroll(&scroll_v)
    .w(px(middle_w))
    .h_full()
    .min_h_0()
}

/// 中间列回滚按钮：放在 hunk 中点行、水平居中（仿 VSCode；仅 enable_discard 时渲染到此）
fn render_middle_revert(hunk_idx: usize, cx: &mut Context<VcsView>) -> AnyElement {
    h_flex()
        .w_full()
        .h(px(DIFF_ROW_H))
        .items_center()
        .justify_center()
        .child(
            Button::new(SharedString::from(format!("vcs-hunk-discard-{hunk_idx}")))
                .ghost()
                .xsmall()
                .icon(gpui_component::IconName::Undo)
                .tooltip("回滚此 hunk（Staged→unstage / Unstaged→discard）")
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.discard_hunk(hunk_idx, cx);
                })),
        )
        .into_any_element()
}

/// 中间列配对行：左列=旧侧作者、右列=新侧作者（删除行的旧作者需历史 blame，暂空）
fn render_middle_cell(
    left_author: Option<String>,
    right_author: Option<String>,
    muted_fg: gpui::Hsla,
) -> AnyElement {
    let col = |author: Option<String>| {
        div()
            .flex_1()
            .min_w_0()
            .px(px(3.0))
            .text_xs()
            .text_color(muted_fg)
            .overflow_hidden()
            .text_ellipsis()
            .whitespace_nowrap()
            .child(author.unwrap_or_default())
    };
    let mut sep = muted_fg;
    sep.a = 0.25;
    h_flex()
        .w_full()
        .h(px(DIFF_ROW_H))
        .items_center()
        .child(col(left_author))
        .child(div().flex_none().w(px(1.0)).h(px(DIFF_ROW_H)).bg(sep))
        .child(col(right_author))
        .into_any_element()
}
