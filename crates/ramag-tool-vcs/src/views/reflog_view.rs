//! Reflog 视图：每行 [hash] HEAD@{N} | action | subject | 时间 + 行尾 [Checkout]
//!
//! 与 history view 互斥（showing_reflog=true 时由 history_panel 调用）。
//! 行级虚拟化：用 `uniform_list`（28px 等高）让上千条 reflog 也滚动流畅。
//! 点击行尾按钮 → checkout 到该 commit（detached HEAD），用户可手动建分支保留。

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, px, uniform_list,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use ramag_domain::entities::ReflogEntry;

use super::vcs_view::VcsView;

/// 每行高度（与 commit 行 28px 对齐，视觉一致）
const ROW_HEIGHT: f32 = 28.0;

impl VcsView {
    /// reflog 视图主入口
    pub(super) fn render_reflog_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let count = self.reflog_entries.len();

        if self.loading_reflog {
            return center("加载 reflog...", muted_fg);
        }
        if self.reflog_entries.is_empty() {
            return center("(reflog 为空)", muted_fg);
        }

        // Rc 共享 reflog 数据给闭包，避免每帧 clone 整个 Vec
        let entries_rc: Rc<Vec<ReflogEntry>> = Rc::new(self.reflog_entries.clone());
        let busy = self.busy;
        let mono = theme.mono_font_family.clone();
        let fg = theme.foreground;
        let accent = theme.accent;

        let body = uniform_list(
            "vcs-reflog-rows",
            count,
            cx.processor({
                let entries_rc = entries_rc.clone();
                let mono = mono.clone();
                move |_this, range: Range<usize>, _w, cx| {
                    let muted_fg = cx.theme().muted_foreground;
                    let hover_bg = cx.theme().muted;
                    range
                        .map(|i| {
                            render_reflog_row(
                                i,
                                &entries_rc[i],
                                busy,
                                fg,
                                muted_fg,
                                accent,
                                hover_bg,
                                mono.clone(),
                                cx,
                            )
                        })
                        .collect::<Vec<_>>()
                }
            }),
        )
        .track_scroll(&self.reflog_scroll)
        .flex_1();

        v_flex()
            .size_full()
            .min_h_0()
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(muted_fg)
                    .pb(px(8.0))
                    .child(format!("HEAD reflog 共 {count} 条")),
            )
            .child(body)
            .into_any_element()
    }
}

/// 单条 reflog 行渲染（在 uniform_list closure 内调）
#[allow(clippy::too_many_arguments)]
fn render_reflog_row(
    idx: usize,
    e: &ReflogEntry,
    busy: bool,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    accent: gpui::Hsla,
    hover_bg: gpui::Hsla,
    mono: SharedString,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let short_hash = if e.commit.0.len() > 7 {
        &e.commit.0[..7]
    } else {
        e.commit.0.as_str()
    };
    let time_str = e.timestamp.format("%m-%d %H:%M").to_string();
    let action_color = match e.action.as_str() {
        "commit" | "commit (initial)" | "commit (amend)" => accent,
        "checkout" => gpui::hsla(220.0 / 360.0, 0.6, 0.55, 1.0),
        "reset" => gpui::hsla(0.0, 0.65, 0.55, 1.0),
        "merge" | "rebase" | "rebase (start)" | "rebase (finish)" => {
            gpui::hsla(280.0 / 360.0, 0.55, 0.55, 1.0)
        }
        _ => muted_fg,
    };
    let commit_for_btn = e.commit.0.clone();
    let row_id = SharedString::from(format!("vcs-reflog-row-{idx}"));

    h_flex()
        .id(row_id)
        .h(px(ROW_HEIGHT))
        .flex_none()
        .gap(px(8.0))
        .items_center()
        .px(px(6.0))
        .rounded(px(3.0))
        .hover(move |this| this.bg(hover_bg))
        .child(
            div()
                .flex_none()
                .w(px(70.0))
                .font_family(mono.clone())
                .text_xs()
                .text_color(accent)
                .child(short_hash.to_string()),
        )
        .child(
            div()
                .flex_none()
                .w(px(86.0))
                .font_family(mono.clone())
                .text_xs()
                .text_color(muted_fg)
                .child(e.selector.clone()),
        )
        .child(
            div()
                .flex_none()
                .w(px(72.0))
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(action_color)
                .child(e.action.clone()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(fg)
                .overflow_hidden()
                .text_ellipsis()
                .child(e.subject.clone()),
        )
        .child(
            div()
                .flex_none()
                .w(px(80.0))
                .text_xs()
                .text_color(muted_fg)
                .font_family(mono)
                .child(time_str),
        )
        .child(
            Button::new(SharedString::from(format!("vcs-reflog-checkout-{idx}")))
                .ghost()
                .xsmall()
                .icon(gpui_component::IconName::ArrowRight)
                .tooltip("Checkout 到此 commit（detached HEAD）")
                .disabled(busy)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.checkout_reflog_entry(commit_for_btn.clone(), cx);
                })),
        )
        .into_any_element()
}

fn center(msg: &'static str, muted_fg: gpui::Hsla) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(muted_fg)
        .child(msg)
        .into_any_element()
}
