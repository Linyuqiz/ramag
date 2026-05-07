//! merge / cherry-pick 进行中横幅（继续 / 中止）+ 冲突文件行尾按钮（Use Ours / Theirs / 已解决）

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};
use ramag_domain::entities::RepoOperation;

use super::helpers::{ConflictOp, OperationStep};
use super::vcs_view::VcsView;

impl VcsView {
    /// `status.operation = Some(_)` 时显示。Merge/CherryPick/Revert 给「继续 / 中止」；Rebase 多「跳过」
    pub(super) fn render_op_banner(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(op) = self.status.as_ref().and_then(|s| s.operation) else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let danger = theme.danger;
        let mut bg = danger;
        bg.a = 0.15;
        let busy = self.busy;

        let title = match op {
            RepoOperation::Merge => "合并进行中",
            RepoOperation::Rebase => "Rebase 进行中",
            RepoOperation::CherryPick => "Cherry-pick 进行中",
            RepoOperation::Revert => "Revert 进行中",
        };
        let supports_skip = matches!(op, RepoOperation::Rebase);

        h_flex()
            .w_full()
            .items_center()
            .gap(px(10.0))
            .px(px(14.0))
            .py(px(8.0))
            .bg(bg)
            .border_b_1()
            .border_color(theme.border)
            .child(
                Icon::new(ramag_ui::icons::git_merge())
                    .small()
                    .text_color(danger),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.foreground)
                    .child(title),
            )
            .child(
                Button::new("vcs-op-continue")
                    .primary()
                    .small()
                    .icon(IconName::Check)
                    .label("继续")
                    .tooltip("提交解决后的合并 / cherry-pick / rebase")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.confirm_op_step(OperationStep::Continue, window, cx);
                    })),
            )
            .when(supports_skip, |this| {
                this.child(
                    Button::new("vcs-op-skip")
                        .ghost()
                        .small()
                        .icon(IconName::ArrowRight)
                        .label("跳过")
                        .tooltip("跳过当前 commit 继续 rebase 下一个")
                        .disabled(busy)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.confirm_op_step(OperationStep::Skip, window, cx);
                        })),
                )
            })
            .child(
                Button::new("vcs-op-abort")
                    .ghost()
                    .small()
                    .icon(IconName::Close)
                    .label("中止")
                    .tooltip("放弃当前进行中的操作，回到操作前的工作区")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.confirm_op_step(OperationStep::Abort, window, cx);
                    })),
            )
            .into_any_element()
    }
}

/// 冲突文件行尾按钮：[查看冲突][Use Ours][Use Theirs][标记已解决]
pub(super) fn conflict_buttons(
    idx: usize,
    path: &str,
    busy: bool,
    cx: &mut Context<VcsView>,
) -> Vec<AnyElement> {
    let path_for_view = path.to_string();
    let view_btn = {
        let id = SharedString::from(format!("vcs-conflict-view-{idx}-{path}"));
        Button::new(id)
            .ghost()
            .xsmall()
            .icon(ramag_ui::icons::columns_2())
            .tooltip("打开三方冲突编辑器（查看 HEAD vs 对方内容）")
            .disabled(busy)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.open_conflict_editor(path_for_view.clone(), cx);
            }))
            .into_any_element()
    };
    vec![
        view_btn,
        conflict_btn(
            "use-ours",
            idx,
            path,
            "采纳「我们」（HEAD 侧）的版本",
            IconName::ArrowLeft,
            ConflictOp::UseOurs,
            busy,
            cx,
        ),
        conflict_btn(
            "use-theirs",
            idx,
            path,
            "采纳「他们」（对方分支）的版本",
            IconName::ArrowRight,
            ConflictOp::UseTheirs,
            busy,
            cx,
        ),
        conflict_btn(
            "mark-resolved",
            idx,
            path,
            "标记已解决（手动改完文件后点这里）",
            IconName::Check,
            ConflictOp::MarkResolved,
            busy,
            cx,
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn conflict_btn(
    kind: &'static str,
    idx: usize,
    path: &str,
    tooltip: &'static str,
    icon: IconName,
    op: ConflictOp,
    busy: bool,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let id = SharedString::from(format!("vcs-conflict-{kind}-{idx}-{path}"));
    let path_owned = path.to_string();
    Button::new(id)
        .ghost()
        .xsmall()
        .icon(icon)
        .tooltip(tooltip)
        .disabled(busy)
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.run_conflict_op(op, path_owned.clone(), cx);
        }))
        .into_any_element()
}
