//! Blame 视图：短 hash / 作者 / 日期 / 行号 | 内容。同 commit 连续行 metadata 仅首行显示

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, UniformListScrollHandle,
    div, px, uniform_list,
};
use gpui_component::h_flex;
use ramag_domain::entities::BlameLine;

use super::vcs_view::VcsView;

const BLAME_ROW_H: f32 = 20.0;

/// uniform_list 虚拟化。`scroll` 必须由调用方传入，render 时不能 `cx.entity().read(cx)`（已被 mut 借用）
#[allow(clippy::too_many_arguments)]
pub fn render_blame(
    lines: &[BlameLine],
    mono: SharedString,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    muted_bg: gpui::Hsla,
    accent: gpui::Hsla,
    scroll: &UniformListScrollHandle,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    if lines.is_empty() {
        return div()
            .px(px(12.0))
            .py(px(20.0))
            .text_sm()
            .text_color(muted_fg)
            .child("（空文件 / 加载失败）")
            .into_any_element();
    }
    // 预计算每行是否与上一行同 commit（避免每帧重算）
    let lines_rc: Rc<Vec<BlameLine>> = Rc::new(lines.to_vec());
    let same_as_prev: Rc<Vec<bool>> = Rc::new({
        let mut prev: Option<&str> = None;
        let mut out = Vec::with_capacity(lines_rc.len());
        for l in lines_rc.iter() {
            let same = prev.is_some_and(|p| p == l.commit.0.as_str());
            out.push(same);
            prev = Some(l.commit.0.as_str());
        }
        out
    });
    let total = lines_rc.len();
    let scroll = scroll.clone();

    uniform_list(
        "vcs-blame",
        total,
        cx.processor({
            let lines_rc = lines_rc.clone();
            let same_as_prev = same_as_prev.clone();
            let mono = mono.clone();
            move |_this, range: Range<usize>, _w, _cx| {
                range
                    .map(|i| {
                        render_blame_line(
                            &lines_rc[i],
                            same_as_prev[i],
                            mono.clone(),
                            fg,
                            muted_fg,
                            muted_bg,
                            accent,
                        )
                        .into_any_element()
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

/// 单行：[hash 7 字符][作者 14 字符][日期][行号][内容]
fn render_blame_line(
    l: &BlameLine,
    same_as_prev: bool,
    mono: SharedString,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    muted_bg: gpui::Hsla,
    accent: gpui::Hsla,
) -> impl IntoElement {
    let hash_short = if l.commit.0.len() > 7 {
        &l.commit.0[..7]
    } else {
        l.commit.0.as_str()
    };
    let date_str = l.timestamp.format("%Y-%m-%d").to_string();
    let author_short: String = l.author.chars().take(14).collect();

    let (hash_text, author_text, date_text) = if same_as_prev {
        (String::new(), String::new(), String::new())
    } else {
        (hash_short.to_string(), author_short, date_str)
    };

    h_flex()
        .w_full()
        .h(px(BLAME_ROW_H))
        .flex_none()
        .items_center()
        .gap(px(0.0))
        .font_family(mono.clone())
        .text_xs()
        .child(
            div()
                .flex_none()
                .w(px(76.0))
                .px(px(6.0))
                .bg(muted_bg)
                .text_color({
                    let mut c = accent;
                    c.a = 0.85;
                    c
                })
                .child(hash_text),
        )
        .child(
            div()
                .flex_none()
                .w(px(120.0))
                .px(px(6.0))
                .bg(muted_bg)
                .text_color(muted_fg)
                .overflow_hidden()
                .text_ellipsis()
                .child(author_text),
        )
        .child(
            div()
                .flex_none()
                .w(px(90.0))
                .px(px(6.0))
                .bg(muted_bg)
                .text_color(muted_fg)
                .child(date_text),
        )
        .child(
            div()
                .flex_none()
                .w(px(50.0))
                .px(px(6.0))
                .text_color(muted_fg)
                .child(l.line_no.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .px(px(6.0))
                .text_color(fg)
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .child(l.content.clone()),
        )
}
