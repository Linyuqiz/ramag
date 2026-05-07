//! Files toolbar 远端操作：dropdown（Fetch / Pull / Push / 强推）+ 分支徽标

#![allow(dead_code)]

use gpui::{AnyElement, Context, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
};

use super::helpers::RemoteOp;
use super::vcs_view::VcsView;

impl VcsView {
    /// HEAD 分支徽标：[git-branch] 分支名 [↑N pill] [↓N pill] [op pill]
    pub(super) fn render_head_badge(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let accent = theme.accent;
        let mut chip_bg = accent;
        chip_bg.a = 0.14;

        let Some(status) = &self.status else {
            return div().into_any_element();
        };

        let branch = status
            .head_branch
            .clone()
            .unwrap_or_else(|| "(detached)".into());
        let ahead = status.ahead.unwrap_or(0);
        let behind = status.behind.unwrap_or(0);

        let mut row = h_flex()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .py(px(2.0))
            .rounded(px(12.0))
            .bg(chip_bg)
            .child(
                Icon::new(ramag_ui::icons::git_branch())
                    .small()
                    .text_color(accent),
            )
            .child(
                div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(fg)
                    .child(branch),
            );

        if ahead > 0 {
            row = row.child(pill_badge(format!("↑{ahead}"), theme.warning));
        }
        if behind > 0 {
            row = row.child(pill_badge(format!("↓{behind}"), theme.danger));
        }
        // 进行中操作（Merge/Rebase/CherryPick/Revert）已由 op_banner 显眼展示，
        // 此处不再贴 pill，避免与 banner 信息重复

        row.into_any_element()
    }

    /// Git 操作聚合：dropdown（Fetch / Pull / Push / 强推）。Pull / Push 按 ahead/behind 显示数字
    pub(super) fn render_remote_actions(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.repo.is_none() {
            return div().into_any_element();
        }
        let busy = self.busy;
        let ahead = self.status.as_ref().and_then(|s| s.ahead).unwrap_or(0);
        let behind = self.status.as_ref().and_then(|s| s.behind).unwrap_or(0);
        let entity = cx.entity();

        let pull_label = if behind > 0 {
            format!("Pull ↓{behind}")
        } else {
            "Pull".into()
        };
        let push_label = if ahead > 0 {
            format!("Push ↑{ahead}")
        } else {
            "Push".into()
        };

        Button::new("vcs-ops-menu")
            .ghost()
            .xsmall()
            .icon(IconName::EllipsisVertical)
            .tooltip("Git 操作（Fetch / Pull / Push / 强推）")
            .disabled(busy)
            .dropdown_menu_with_anchor(gpui::Anchor::BottomRight, move |mut m, _, _| {
                let entity1 = entity.clone();
                let entity2 = entity.clone();
                let entity3 = entity.clone();
                let entity4 = entity.clone();
                m = m
                    .item(PopupMenuItem::new("Fetch").on_click(move |_, _, app| {
                        entity1.update(app, |this, cx| {
                            this.run_remote_op(RemoteOp::Fetch, cx);
                        });
                    }))
                    .item(
                        PopupMenuItem::new(pull_label.clone()).on_click(move |_, _, app| {
                            entity2.update(app, |this, cx| {
                                this.run_remote_op(RemoteOp::Pull, cx);
                            });
                        }),
                    )
                    .item(
                        PopupMenuItem::new(push_label.clone()).on_click(move |_, _, app| {
                            entity3.update(app, |this, cx| {
                                this.run_remote_op(RemoteOp::Push, cx);
                            });
                        }),
                    )
                    .item(PopupMenuItem::new("⚠ 强推").on_click(move |_, w, app| {
                        entity4.update(app, |this, cx| {
                            this.confirm_remote_op(RemoteOp::PushForce, w, cx);
                        });
                    }));
                m
            })
            .into_any_element()
    }
}

/// pill 徽标：「↑1」「↓2」「Merge」等；tone 决定语义高亮色（warning/danger）
fn pill_badge(text: String, tone: gpui::Hsla) -> AnyElement {
    let mut bg = tone;
    bg.a = 0.18;
    div()
        .px(px(6.0))
        .py(px(1.0))
        .rounded(px(8.0))
        .bg(bg)
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(tone)
        .child(text)
        .into_any_element()
}
