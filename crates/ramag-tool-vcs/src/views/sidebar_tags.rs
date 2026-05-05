//! 左侧边栏：Tag 段
//!
//! 列表显示 tag 名 + 短 hash + （annotated 才有的）message 一行预览，
//! 行尾按钮 [Push][Delete]。底部有「新建 tag」输入 + 创建按钮（lightweight 模式）。

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    v_flex,
};
use ramag_domain::entities::{Tag, TagKind};

use super::helpers::TagOp;
use super::sidebar::{SidebarSection, section_header};
use super::vcs_view::VcsView;

impl VcsView {
    /// Tag 段：折叠 + 列表 + 行尾按钮 + 底部新建
    pub(super) fn render_tags_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let count = self.tags.len();
        let busy = self.busy;
        let collapsed = self.collapsed_tag;

        let header = section_header("Tag", count, collapsed, SidebarSection::Tag, cx);
        if collapsed {
            return v_flex().gap(px(4.0)).child(header).into_any_element();
        }

        let body: AnyElement = if self.loading_tags {
            div()
                .pl(px(4.0))
                .text_xs()
                .text_color(muted_fg)
                .child("加载中...")
                .into_any_element()
        } else if self.tags.is_empty() {
            div()
                .pl(px(4.0))
                .text_xs()
                .text_color(muted_fg)
                .child("(无 tag)")
                .into_any_element()
        } else {
            let rows: Vec<AnyElement> = self
                .tags
                .iter()
                .enumerate()
                .map(|(i, t)| tag_row(i, t, busy, cx).into_any_element())
                .collect();
            v_flex().gap(px(1.0)).children(rows).into_any_element()
        };

        // 底部「新建 tag」：name 一行 + message 一行 + 创建按钮
        // message 非空 → annotated tag；空 → lightweight tag
        let create_row = v_flex()
            .gap(px(4.0))
            .pt(px(4.0))
            .child(
                Input::new(&self.create_tag_input)
                    .xsmall()
                    .into_any_element(),
            )
            .child(
                h_flex()
                    .gap(px(4.0))
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
                    ),
            );

        v_flex()
            .gap(px(2.0))
            .child(header)
            .child(body)
            .child(create_row)
            .into_any_element()
    }
}

/// 单条 tag 行：[tag-icon] name + (msg | hash) + 行尾 [Push][Delete]
fn tag_row(idx: usize, t: &Tag, busy: bool, cx: &mut Context<VcsView>) -> impl IntoElement {
    let theme = cx.theme();
    let fg = theme.foreground;
    let muted_fg = theme.muted_foreground;
    let mono = theme.mono_font_family.clone();
    let hover_bg = theme.muted;
    // 暖橙色：tag 与分支区分（与 commit row 内的 tag chip 同色系）
    let tag_color = gpui::hsla(40.0 / 360.0, 0.7, 0.55, 1.0);

    let kind_label = match t.kind {
        TagKind::Annotated => "A",
        TagKind::Lightweight => "L",
    };
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
        .gap(px(6.0))
        .items_center()
        .py(px(3.0))
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
            v_flex()
                .gap(px(0.0))
                .flex_1()
                .min_w_0()
                .child(
                    h_flex()
                        .gap(px(4.0))
                        .items_baseline()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(fg)
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(name.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .child(format!("({kind_label})")),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted_fg)
                        .font_family(mono)
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(detail),
                ),
        )
        .child(
            h_flex()
                .gap(px(2.0))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
                .child(tag_btn(
                    "push",
                    idx,
                    "推送 tag 到 origin",
                    IconName::ArrowUp,
                    TagOp::Push(name.clone()),
                    busy,
                    cx,
                ))
                .child(tag_btn_icon(
                    "delete",
                    idx,
                    "删除本地 tag",
                    ramag_ui::icons::trash(),
                    TagOp::Delete(name),
                    busy,
                    cx,
                )),
        )
}

fn tag_btn(
    kind: &'static str,
    idx: usize,
    tooltip: &'static str,
    icon: IconName,
    op: TagOp,
    busy: bool,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let id = SharedString::from(format!("vcs-side-tag-{kind}-{idx}"));
    Button::new(id)
        .ghost()
        .xsmall()
        .icon(icon)
        .tooltip(tooltip)
        .disabled(busy)
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.confirm_tag_op(op.clone(), window, cx);
        }))
        .into_any_element()
}

fn tag_btn_icon(
    kind: &'static str,
    idx: usize,
    tooltip: &'static str,
    icon: Icon,
    op: TagOp,
    busy: bool,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let id = SharedString::from(format!("vcs-side-tag-{kind}-{idx}"));
    Button::new(id)
        .ghost()
        .xsmall()
        .icon(icon)
        .tooltip(tooltip)
        .disabled(busy)
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.confirm_tag_op(op.clone(), window, cx);
        }))
        .into_any_element()
}
