//! Stash 列表渲染（IDE Files panel 的 Stash 视图主区调用）
//!
//! 行尾按钮 [Apply][Pop][Drop]，每条 stash 显示 stash@{N} + message + 时间。

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use ramag_domain::entities::Stash;

use super::helpers::StashOp;
use super::vcs_view::VcsView;

impl VcsView {
    /// Stash 列表 body：供 IDE Files panel Stash 视图主区调用
    pub(super) fn render_stash_list_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted_fg = cx.theme().muted_foreground;
        let busy = self.busy;
        if self.loading_stashes {
            return div()
                .pl(px(4.0))
                .text_xs()
                .text_color(muted_fg)
                .child("加载中...")
                .into_any_element();
        }
        if self.stashes.is_empty() {
            return div()
                .pl(px(4.0))
                .text_xs()
                .text_color(muted_fg)
                .child("(无 stash)")
                .into_any_element();
        }
        let rows: Vec<AnyElement> = self
            .stashes
            .iter()
            .map(|s| stash_row(s, busy, cx).into_any_element())
            .collect();
        v_flex().gap(px(2.0)).children(rows).into_any_element()
    }
}

/// 单条 stash 行：紧凑布局 stash@{N} + msg + 行尾按钮
fn stash_row(s: &Stash, busy: bool, cx: &mut Context<VcsView>) -> impl IntoElement {
    let theme = cx.theme();
    let fg = theme.foreground;
    let muted_fg = theme.muted_foreground;
    let mono = theme.mono_font_family.clone();
    let idx = s.id.0;

    v_flex()
        .gap(px(2.0))
        .py(px(3.0))
        .px(px(4.0))
        .rounded(px(3.0))
        .child(
            h_flex()
                .gap(px(6.0))
                .items_baseline()
                .child(
                    div()
                        .flex_none()
                        .font_family(mono)
                        .text_xs()
                        .text_color(theme.accent)
                        .child(format!("stash@{{{idx}}}")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_xs()
                        .text_color(fg)
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(s.message.clone()),
                ),
        )
        .child(
            h_flex()
                .gap(px(4.0))
                .items_center()
                .child(stash_btn(
                    "apply",
                    idx,
                    "应用（保留 stash）",
                    IconName::ArrowDown,
                    StashOp::Apply(idx),
                    busy,
                    cx,
                ))
                .child(stash_btn(
                    "pop",
                    idx,
                    "应用并删除 stash",
                    IconName::Check,
                    StashOp::Pop(idx),
                    busy,
                    cx,
                ))
                .child(stash_btn_icon(
                    "drop",
                    idx,
                    "丢弃 stash",
                    ramag_ui::icons::trash(),
                    StashOp::Drop(idx),
                    busy,
                    cx,
                ))
                .child(div().flex_1())
                .child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(muted_fg)
                        .child(s.timestamp.format("%m-%d %H:%M").to_string()),
                ),
        )
}

fn stash_btn(
    kind: &'static str,
    idx: usize,
    tooltip: &'static str,
    icon: IconName,
    op: StashOp,
    busy: bool,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let id = SharedString::from(format!("vcs-side-stash-{kind}-{idx}"));
    Button::new(id)
        .ghost()
        .xsmall()
        .icon(icon)
        .tooltip(tooltip)
        .disabled(busy)
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.confirm_stash_op(op, window, cx);
        }))
        .into_any_element()
}

fn stash_btn_icon(
    kind: &'static str,
    idx: usize,
    tooltip: &'static str,
    icon: gpui_component::Icon,
    op: StashOp,
    busy: bool,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let id = SharedString::from(format!("vcs-side-stash-{kind}-{idx}"));
    Button::new(id)
        .ghost()
        .xsmall()
        .icon(icon)
        .tooltip(tooltip)
        .disabled(busy)
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.confirm_stash_op(op, window, cx);
        }))
        .into_any_element()
}
