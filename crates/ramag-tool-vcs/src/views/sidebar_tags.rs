//! 侧栏 Tag：单行 tag 行（名字 + 详情内联 + Push/Delete）+ 底部新建 tag 输入行。
//! 行由 history 左栏的单个 uniform_list 统一渲染（28px 等高），段组装见 history_panel

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
};
use ramag_domain::entities::Tag;

use super::helpers::{TagOp, side_op_button};
use super::sidebar::LEFT_ROW_H;
use super::vcs_view::VcsView;

impl VcsView {
    /// 底部「新建 tag」输入行：name 一格 + message 一格 + 创建按钮（固定 28px 高，单行）
    /// message 非空 → annotated tag；空 → lightweight tag
    pub(super) fn render_create_tag_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let busy = self.busy;
        h_flex()
            .h(px(LEFT_ROW_H))
            .flex_none()
            .gap(px(4.0))
            .items_center()
            .child(
                div().flex_none().w(px(90.0)).child(
                    Input::new(&self.create_tag_input)
                        .xsmall()
                        .into_any_element(),
                ),
            )
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.create_tag_message_input)
                        .xsmall()
                        .into_any_element(),
                ),
            )
            .child(
                Button::new("vcs-tag-create")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .tooltip("创建 tag（message 非空 → annotated；空 → lightweight）")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.handle_create_tag(cx);
                    })),
            )
            .into_any_element()
    }
}

/// 单条 tag 行：[tag-icon] name + 详情（message / 短 hash）内联 + 行尾 [Push][Delete]（固定 28px 高）
pub(super) fn tag_row(
    idx: usize,
    t: &Tag,
    busy: bool,
    cx: &mut Context<VcsView>,
) -> impl IntoElement {
    let theme = cx.theme();
    let fg = theme.foreground;
    let muted_fg = theme.muted_foreground;
    let mono = theme.mono_font_family.clone();
    let hover_bg = theme.muted;
    // 暖橙色：tag 与分支区分（与 commit row 内的 tag chip 同色系）
    let tag_color = gpui::hsla(40.0 / 360.0, 0.7, 0.55, 1.0);

    // 有 message 就显示（annotated = tag 自己的 message；lightweight = commit subject），
    // 都没有时回退到 commit hash
    let detail = match &t.message {
        Some(m) => m.clone(),
        None => t.commit.short().to_string(),
    };
    let name = t.name.clone();
    let row_id = SharedString::from(format!("vcs-side-tag-{idx}-{name}"));

    h_flex()
        .id(row_id)
        .h(px(LEFT_ROW_H))
        .flex_none()
        .gap(px(6.0))
        .items_center()
        .px(px(4.0))
        .rounded(px(3.0))
        .hover(move |this| this.bg(hover_bg))
        .child(
            div().flex_none().w(px(14.0)).child(
                Icon::new(ramag_ui::icons::circle_dot())
                    .xsmall()
                    .text_color(tag_color),
            ),
        )
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(px(6.0))
                .items_baseline()
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .child(name.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_xs()
                        .font_family(mono)
                        .text_color(muted_fg)
                        .child(detail),
                ),
        )
        .child(
            h_flex()
                .gap(px(2.0))
                .flex_none()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
                .child({
                    let name = name.clone();
                    side_op_button(
                        format!("vcs-side-tag-push-{idx}"),
                        "推送 tag 到 origin",
                        IconName::ArrowUp,
                        busy,
                        move |this, window, cx| {
                            this.confirm_tag_op(TagOp::Push(name.clone()), window, cx)
                        },
                        cx,
                    )
                })
                .child(side_op_button(
                    format!("vcs-side-tag-delete-{idx}"),
                    "删除本地 tag",
                    ramag_ui::icons::trash(),
                    busy,
                    move |this, window, cx| {
                        this.confirm_tag_op(TagOp::Delete(name.clone()), window, cx)
                    },
                    cx,
                )),
        )
}
