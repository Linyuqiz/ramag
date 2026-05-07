//! 工作区底部 commit 面板：subject 输入 + amend 切换 + 提交按钮

use gpui::{AnyElement, ClickEvent, Context, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
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
        let can_commit = !self.busy && (staged_count > 0 || self.commit_amend);

        let amend_btn = Button::new("vcs-amend-toggle")
            .ghost()
            .small()
            .icon(IconName::Undo)
            .label(if self.commit_amend {
                "Amend ✓"
            } else {
                "Amend"
            })
            .tooltip("修改上一次 commit（不创建新 commit）")
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.commit_amend = !this.commit_amend;
                cx.notify();
            }));

        let sign_btn = Button::new("vcs-commit-sign-toggle")
            .ghost()
            .small()
            .icon(IconName::CircleCheck)
            .label(if self.commit_sign { "Sign ✓" } else { "Sign" })
            .tooltip("用 GPG 签名 commit（需配好 user.signingkey 和 GPG agent）")
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.commit_sign = !this.commit_sign;
                cx.notify();
            }));

        let commit_btn = Button::new("vcs-commit")
            .primary()
            .small()
            .icon(ramag_ui::icons::git_commit())
            .label(if staged_count > 0 {
                format!("提交 ({staged_count})")
            } else {
                "提交".to_string()
            })
            .tooltip("把暂存区的改动写入仓库")
            .disabled(!can_commit)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.confirm_commit(window, cx);
            }));

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
                    .gap(px(8.0))
                    .child(sign_btn)
                    .child(amend_btn)
                    .child(commit_btn),
            )
            .into_any_element()
    }
}
