//! 左侧边栏：远程仓库（remote）管理段
//!
//! 注：sidebar 整体面板已删除（IDEA 三栏接管），此模块代码暂保留备查 / 复用。
//! 整套 remote 配置 UI（add / remove / setUrl）目前无入口——未来若需重新启用，
//! 把 `render_remote_repo_section` 挂到某个新位置即可。

#![allow(dead_code)]

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
use ramag_domain::entities::Remote;

use super::sidebar::{SidebarSection, section_header};
use super::vcs_view::VcsView;
use super::vcs_view_ops_remote::RemoteAdminOp;

impl VcsView {
    /// 远程仓库段：折叠 + 列表 + 底部 add（name+url）
    pub(super) fn render_remote_repo_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let count = self.remotes.len();
        let busy = self.busy;
        let collapsed = self.collapsed_remote_section;

        let header = section_header("远程仓库", count, collapsed, SidebarSection::RemoteRepo, cx);
        if collapsed {
            return v_flex().gap(px(4.0)).child(header).into_any_element();
        }

        let body: AnyElement = if self.loading_remotes {
            div()
                .pl(px(4.0))
                .text_xs()
                .text_color(muted_fg)
                .child("加载中...")
                .into_any_element()
        } else if self.remotes.is_empty() {
            div()
                .pl(px(4.0))
                .text_xs()
                .text_color(muted_fg)
                .child("(未配置 remote)")
                .into_any_element()
        } else {
            let rows: Vec<AnyElement> = self
                .remotes
                .iter()
                .enumerate()
                .map(|(i, r)| remote_row(i, r, busy, cx).into_any_element())
                .collect();
            v_flex().gap(px(2.0)).children(rows).into_any_element()
        };

        // 底部「添加 remote」：两栏输入（name 短 / url 长）
        let add_row = v_flex()
            .gap(px(4.0))
            .pt(px(4.0))
            .child(
                Input::new(&self.add_remote_name_input)
                    .xsmall()
                    .into_any_element(),
            )
            .child(
                h_flex()
                    .gap(px(4.0))
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.add_remote_url_input)
                                .xsmall()
                                .into_any_element(),
                        ),
                    )
                    .child(
                        Button::new("vcs-remote-add")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .tooltip("添加 remote 配置")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.handle_add_remote(cx);
                            })),
                    ),
            );

        v_flex()
            .gap(px(2.0))
            .child(header)
            .child(body)
            .child(add_row)
            .into_any_element()
    }
}

/// 单条 remote 行：[icon] name + URL（小灰）+ [删除]
fn remote_row(idx: usize, r: &Remote, busy: bool, cx: &mut Context<VcsView>) -> impl IntoElement {
    let theme = cx.theme();
    let fg = theme.foreground;
    let muted_fg = theme.muted_foreground;
    let mono = theme.mono_font_family.clone();
    let hover_bg = theme.muted;
    let accent = theme.accent;

    let display_url = match &r.push_url {
        Some(p) => format!("{} (push: {p})", r.fetch_url),
        None => r.fetch_url.clone(),
    };
    let row_id = SharedString::from(format!("vcs-side-remote-{idx}-{}", r.name));
    let name_for_btn = r.name.clone();

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
                Icon::new(ramag_ui::icons::download())
                    .xsmall()
                    .text_color(accent),
            ),
        )
        .child(
            v_flex()
                .gap(px(0.0))
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(r.name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted_fg)
                        .font_family(mono)
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(display_url),
                ),
        )
        .child(
            h_flex()
                .gap(px(2.0))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                })
                .child(
                    Button::new(SharedString::from(format!("vcs-remote-del-{idx}")))
                        .ghost()
                        .xsmall()
                        .icon(ramag_ui::icons::trash())
                        .tooltip("删除此 remote 配置")
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.confirm_remote_admin_op(
                                RemoteAdminOp::Remove(name_for_btn.clone()),
                                window,
                                cx,
                            );
                        })),
                ),
        )
}
