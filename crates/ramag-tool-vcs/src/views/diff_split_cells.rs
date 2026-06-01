//! Split diff 单元格：gutter（钉死，含 marker/lineno）+ content（横滚，仅代码 / hunk 头 / spacer）

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement,
    SharedString, Styled, div, prelude::*, px,
};
use gpui_component::h_flex;
use ramag_domain::entities::{DiffLine, DiffLineKind};

use super::diff_panel::{
    DIFF_ROW_H, SPLIT_MARKER_W, line_no_cell, line_no_cell_clickable, line_palette,
};
use super::vcs_view::VcsView;

/// gutter 单元格：左栏 `[marker][lineno]`；右栏 `[lineno][marker]`（blame 移至中间列）
pub(super) fn render_gutter_cell(
    side: &'static str,
    line: Option<(usize, &DiffLine)>,
    hunk_idx: usize,
    is_left: bool,
    muted_fg: gpui::Hsla,
    mono: SharedString,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let Some((line_idx, line)) = line else {
        // 空行（对侧专属）：淡灰背景标识「此处无对应行」，避免被误读成渲染缺失
        let mut empty = h_flex()
            .h(px(DIFF_ROW_H))
            .bg(gpui::hsla(0.0, 0.0, 0.5, 0.05));
        let marker_slot = div().flex_none().w(px(SPLIT_MARKER_W));
        if is_left {
            empty = empty
                .child(marker_slot)
                .child(line_no_cell(String::new(), muted_fg));
        } else {
            empty = empty
                .child(line_no_cell(String::new(), muted_fg))
                .child(marker_slot);
        }
        return empty.into_any_element();
    };

    let (_bg, _, marker_color) = line_palette(line.kind);
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
        .font_family(mono)
        .child(match line.kind {
            DiffLineKind::Add => "+",
            DiffLineKind::Delete => "-",
            DiffLineKind::Context => " ",
        });

    let mut row = h_flex().id(row_id).h(px(DIFF_ROW_H)).text_xs();
    if is_left {
        row = row.child(marker_div).child(lineno_div);
    } else {
        row = row.child(lineno_div).child(marker_div);
    }
    row.into_any_element()
}

/// content 单元格：渲染代码文本（按 lang 语法高亮），宽度由外层 list `w(content_w)` 撑开
#[allow(clippy::too_many_arguments)]
pub(super) fn render_content_cell(
    side: &'static str,
    line: Option<(usize, &DiffLine)>,
    hunk_idx: usize,
    lang: Option<&str>,
    fg: gpui::Hsla,
    mono: SharedString,
    content_w: f32,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let Some((line_idx, line)) = line else {
        return h_flex()
            .h(px(DIFF_ROW_H))
            .min_w(px(content_w))
            .bg(gpui::hsla(0.0, 0.0, 0.5, 0.05))
            .into_any_element();
    };
    let (bg, _, _) = line_palette(line.kind);
    let row_id = SharedString::from(format!("vcs-diff-cnt-{side}-{hunk_idx}-{line_idx}"));

    let text_div =
        div()
            .flex_1()
            .min_w(px(content_w))
            .px(px(4.0))
            .child(super::syntax::render_code_line(
                &line.text, lang, fg, mono, cx,
            ));

    let mut row = h_flex()
        .id(row_id)
        .h(px(DIFF_ROW_H))
        .min_w(px(content_w))
        .child(text_div);
    if let Some(c) = bg {
        row = row.bg(c);
    }
    row.into_any_element()
}

/// gutter hunk header：仅 muted_bg 占位行（回滚按钮已移至中间列）
pub(super) fn render_gutter_header(muted_bg: gpui::Hsla) -> AnyElement {
    h_flex()
        .w_full()
        .h(px(DIFF_ROW_H))
        .flex_none()
        .bg(muted_bg)
        .into_any_element()
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
    let row_id = SharedString::from(format!("vcs-diff-spacer-{side}-{hunk_idx}-{run_start}"));
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
        .child(format!("───── 跳过 {skipped} 行未变更（点击展开） ─────"))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.expanded_diff_spacers.insert((hunk_idx, run_start));
            cx.notify();
        }))
        .into_any_element()
}
