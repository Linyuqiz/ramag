//! 左侧边栏：本地分支段 + 远程分支段
//!
//! 分支行只显示名字 + 上游同步信息，操作全部移至右键菜单。
//! 本地段底部保留「新建分支」输入框 + 创建按钮。

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem},
    v_flex,
};
use ramag_domain::entities::Branch;

use super::confirm_dialogs::open_confirm_dialog;
use super::helpers::BranchOp;
use super::sidebar::{SidebarSection, section_header};
use super::vcs_view::VcsView;

impl VcsView {
    /// 本地分支段：title + 折叠 + 列表 + 底部新建输入
    pub(super) fn render_local_branches_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self.local_branches.len();
        let busy = self.busy;
        let collapsed = self.collapsed_local;

        let header = section_header("本地分支", count, collapsed, SidebarSection::Local, cx);
        if collapsed {
            return v_flex().gap(px(4.0)).child(header).into_any_element();
        }

        let rows: Vec<AnyElement> = self
            .local_branches
            .iter()
            .enumerate()
            .map(|(idx, b)| branch_row(idx, b, busy, false, cx).into_any_element())
            .collect();

        let create_row = h_flex()
            .gap(px(4.0))
            .pt(px(4.0))
            .items_center()
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.create_branch_input)
                        .xsmall()
                        .into_any_element(),
                ),
            )
            .child(
                Button::new("vcs-branch-create")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .tooltip("基于当前 HEAD 创建本地分支")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.handle_create_branch(cx);
                    })),
            );

        v_flex()
            .gap(px(2.0))
            .child(header)
            .child(v_flex().gap(px(1.0)).children(rows))
            .child(create_row)
            .into_any_element()
    }

    /// 远程分支段（只读列表，无创建操作；默认折叠）
    pub(super) fn render_remote_branches_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let count = self.remote_branches.len();
        let busy = self.busy;
        let collapsed = self.collapsed_remote;

        let header = section_header("远程分支", count, collapsed, SidebarSection::Remote, cx);
        if collapsed {
            return v_flex().gap(px(4.0)).child(header).into_any_element();
        }

        let body: AnyElement = if self.remote_branches.is_empty() {
            div()
                .pl(px(4.0))
                .text_xs()
                .text_color(muted_fg)
                .child("(无远程分支)")
                .into_any_element()
        } else {
            let rows: Vec<AnyElement> = self
                .remote_branches
                .iter()
                .enumerate()
                .map(|(idx, b)| branch_row(idx, b, busy, true, cx).into_any_element())
                .collect();
            v_flex().gap(px(1.0)).children(rows).into_any_element()
        };

        v_flex()
            .gap(px(2.0))
            .child(header)
            .child(body)
            .into_any_element()
    }
}

/// 单条分支行：图标 + 名字 + 上游同步；操作通过右键菜单触发
fn branch_row(
    idx: usize,
    b: &Branch,
    busy: bool,
    is_remote: bool,
    cx: &mut Context<VcsView>,
) -> impl IntoElement {
    let theme = cx.theme();
    let fg = theme.foreground;
    let muted_fg = theme.muted_foreground;
    let accent = theme.accent;
    let hover_bg = theme.muted;
    let entity = cx.entity();

    let name = b.name.clone();
    let is_head = b.is_head;
    let name_color = if is_head { accent } else { fg };
    let prefix_color = if is_head { accent } else { muted_fg };

    let sync_str = match (b.ahead, b.behind) {
        (Some(a), Some(d)) if a > 0 || d > 0 => Some(format!("↑{a} ↓{d}")),
        _ => None,
    };

    let row_id = SharedString::from(format!("vcs-side-br-{}-{}-{}", idx, is_remote, name));
    let prefix_icon = if is_head {
        Icon::new(ramag_ui::icons::circle_dot())
            .xsmall()
            .text_color(prefix_color)
            .into_any_element()
    } else {
        Icon::new(ramag_ui::icons::git_branch())
            .xsmall()
            .text_color(prefix_color)
            .into_any_element()
    };

    let row = h_flex()
        .id(row_id)
        .gap(px(6.0))
        .items_center()
        .py(px(3.0))
        .px(px(4.0))
        .rounded(px(3.0))
        .hover(move |this| this.bg(hover_bg))
        .child(div().flex_none().w(px(14.0)).child(prefix_icon))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .font_weight(if is_head {
                    gpui::FontWeight::SEMIBOLD
                } else {
                    gpui::FontWeight::NORMAL
                })
                .text_color(name_color)
                .overflow_hidden()
                .text_ellipsis()
                .child(name.clone()),
        )
        .child(
            div()
                .flex_none()
                .text_xs()
                .text_color(muted_fg)
                .child(sync_str.unwrap_or_default()),
        );

    // 右键菜单：checkout / merge / rebase / interactive-rebase / delete
    row.context_menu({
        let ent = entity.clone();
        let n = name.clone();
        move |menu: PopupMenu, _, _| {
            if !is_head {
                let (e1, n1) = (ent.clone(), n.clone());
                let (e2, n2) = (ent.clone(), n.clone());
                let (e3, n3) = (ent.clone(), n.clone());
                let n4 = n.clone();
                let mut m = menu;
                if !is_remote {
                    // 切换
                    m = m.item(PopupMenuItem::new("切换到此分支").on_click(move |_, w, app| {
                        e1.update(app, |this, cx| {
                            this.confirm_branch_op(BranchOp::Checkout(n1.clone()), w, cx);
                        });
                    }));
                } else {
                    m = m.item(PopupMenuItem::new("切到此远程分支（创建本地副本）").on_click(
                        move |_, w, app| {
                            e1.update(app, |this, cx| {
                                this.confirm_branch_op(BranchOp::Checkout(n1.clone()), w, cx);
                            });
                        },
                    ));
                }
                // 合并
                m = m.item(PopupMenuItem::new("合并到当前 HEAD（--no-ff）").on_click(
                    move |_, w, app| {
                        e2.update(app, |this, cx| {
                            this.confirm_branch_op(BranchOp::Merge(n2.clone()), w, cx);
                        });
                    },
                ));
                // Rebase
                m = m.item(PopupMenuItem::new("Rebase 当前 HEAD 到此分支").on_click(
                    move |_, w, app| {
                        e3.update(app, |this, cx| {
                            this.confirm_branch_op(BranchOp::Rebase(n3.clone()), w, cx);
                        });
                    },
                ));
                if !is_remote {
                    // 交互式 Rebase（仅本地分支）
                    let (ei, ni) = (ent.clone(), n.clone());
                    m = m.item(
                        PopupMenuItem::new("交互式 Rebase（编辑 commit 序列）").on_click(
                            move |_, _, app| {
                                if !busy {
                                    ei.update(app, |this, cx| {
                                        this.start_interactive_rebase(ni.clone(), cx);
                                    });
                                }
                            },
                        ),
                    );
                    m = m.separator();
                    // 删除分支
                    let ed = ent.clone();
                    m = m.item(PopupMenuItem::new("删除分支").on_click(move |_, w, app| {
                        let view = ed.clone();
                        let branch_name = n4.clone();
                        open_confirm_dialog(
                            view,
                            "删除分支？",
                            format!("将删除本地分支「{branch_name}」（仅当已合并；未合并会报错）。\n确认继续吗？"),
                            "删除",
                            true,
                            move |this, cx| {
                                this.run_branch_op(BranchOp::Delete(branch_name.clone(), false), cx)
                            },
                            w,
                            app,
                        );
                    }));
                }
                m
            } else {
                menu
            }
        }
    })
}
