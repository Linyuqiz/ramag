//! 工作区底部 commit 面板：subject 输入 + amend 切换 + 提交按钮

use gpui::{AnyElement, ClickEvent, Context, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};

use super::vcs_view::VcsView;

impl VcsView {
    /// commit 面板：底部固定区，subject 输入 + amend toggle + 提交
    pub(super) fn render_commit_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let accent = theme.accent;
        let border = theme.border;

        let staged_count = self
            .status
            .as_ref()
            .map(|s| s.files.iter().filter(|f| f.staged.is_some()).count())
            .unwrap_or(0);
        // 非 amend 必须有 commit message；amend 可沿用上一次 message 故不强制
        let has_message = !self.commit_input.read(cx).value().trim().is_empty();
        let can_commit = !self.busy
            && (staged_count > 0 || self.commit_amend)
            && (has_message || self.commit_amend);

        // 主按钮：普通模式提交暂存区；Amend 模式改写上一次 commit
        let commit_btn = Button::new("vcs-commit")
            .primary()
            .small()
            .icon(ramag_ui::icons::git_commit())
            .label(if self.commit_amend {
                "Amend 提交".to_string()
            } else if staged_count > 0 {
                format!("提交 ({staged_count})")
            } else {
                "提交".to_string()
            })
            .tooltip(if self.commit_amend {
                "改写上一次 commit（message 留空则保留原文）"
            } else {
                "把暂存区的改动写入仓库"
            })
            .disabled(!can_commit)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.confirm_commit(window, cx);
            }));
        // 右侧小箭头：下拉切换 Amend 模式（与主按钮拼成分体按钮）
        let amend_on = self.commit_amend;
        let entity = cx.entity();
        let more_btn = Button::new("vcs-commit-more")
            .primary()
            .small()
            .icon(IconName::ChevronDown)
            .tooltip("更多提交方式")
            .dropdown_menu_with_anchor(gpui::Anchor::BottomRight, move |mut m, _, _| {
                let ent = entity.clone();
                let label = if amend_on {
                    "✓ Amend 模式（点击退出）"
                } else {
                    "Amend 上一次提交"
                };
                m = m.item(PopupMenuItem::new(label).on_click(move |_, _, app| {
                    ent.update(app, |this, cx| this.toggle_commit_amend(cx));
                }));
                m
            });

        v_flex()
            .flex_none()
            .gap(px(4.0))
            .px(px(10.0))
            .py(px(8.0))
            .border_t_1()
            .border_color(border)
            .child(
                h_flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        Icon::new(ramag_ui::icons::git_commit())
                            .small()
                            .text_color(accent),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(accent)
                            .child("Commit"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(if staged_count > 0 {
                                format!("· 已暂存 {staged_count} 个文件")
                            } else {
                                "· 暂存区为空（先 Stage 文件）".to_string()
                            }),
                    ),
            )
            .child(
                Input::new(&self.commit_input)
                    .h(px(72.0))
                    .into_any_element(),
            )
            .child(
                h_flex()
                    .items_center()
                    .justify_end()
                    .gap(px(2.0))
                    .child(commit_btn)
                    .child(more_btn),
            )
            .into_any_element()
    }
}
