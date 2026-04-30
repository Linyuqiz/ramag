//! 单元格编辑弹框
//!
//! 双击结果集单元格触发：弹框带多行 InputState，初值为该 cell 的当前值。
//! 用户编辑后点"确认修改"，异步执行 UPDATE 并同步本地 cell。
//! 失败 / affected_rows=0 时通过 toast 反馈，弹框已关闭，用户可重新打开。
//!
//! 调用约束：必须由 listener 在已持 ResultPanel mut ref 的上下文调用，
//! 数据（col_name + 已建好的 InputState）由调用方预先提供，
//! 本函数内部不调 panel.read(cx)，避免 GPUI 二次借用 panic。

use gpui::{
    ClickEvent, Context, Entity, IntoElement, ParentElement, SharedString, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
};

use super::result_panel::ResultPanel;

/// 打开单元格编辑弹框
/// - col_name：弹框标题里的列名
/// - input：预建多行编辑框（已 default_value）
/// - has_pk：当前结果集是否能推断主键（影响弹框上方提示）
pub(super) fn open(
    panel: Entity<ResultPanel>,
    ri: usize,
    ci: usize,
    col_name: String,
    input: Entity<InputState>,
    has_pk: bool,
    window: &mut Window,
    cx: &mut Context<ResultPanel>,
) {
    let title: SharedString = format!("编辑 行 {} · {}", ri + 1, col_name).into();

    // 弹框打开后立即让 InputState 拿到焦点，用户不用再点一下输入框
    input.update(cx, |state, cx_inner| {
        state.focus(window, cx_inner);
    });

    // dialog build 闭包是 Fn（每次重渲染都调），需要在外面 clone 一份给闭包
    let panel_for_dialog = panel.clone();
    let input_for_dialog = input.clone();

    window.open_dialog(cx, move |dialog, _, _| {
        let panel_btn = panel_for_dialog.clone();
        let input_btn = input_for_dialog.clone();

        let cancel_btn = Button::new("cell-edit-cancel")
            .ghost()
            .small()
            .label("取消")
            .on_click({
                let panel = panel_btn.clone();
                move |_: &ClickEvent, window, app| {
                    panel.update(app, |this, _| this.set_cell_edit_input(None));
                    window.close_dialog(app);
                }
            });

        let apply_btn = Button::new("cell-edit-apply")
            .primary()
            .small()
            .label("确认")
            .on_click({
                let panel = panel_btn.clone();
                let input = input_btn.clone();
                move |_: &ClickEvent, window, app| {
                    let new_val = input.read(app).value().to_string();
                    panel.update(app, |this, cx_inner| {
                        this.apply_cell_update_async(ri, ci, new_val, cx_inner);
                        this.set_cell_edit_input(None);
                    });
                    window.close_dialog(app);
                }
            });

        let input_for_content = input_for_dialog.clone();
        dialog
            .title(title.clone())
            // 显式宽度让 Dialog 在水平方向居中（gpui-component 内部用 width/2 算 x）
            .width(px(560.0))
            .margin_top(px(140.0))
            .content(move |content, _, cx| {
                let theme = cx.theme();
                let muted_fg = theme.muted_foreground;
                let warning = theme.warning;
                let hint: gpui::AnyElement = if has_pk {
                    div()
                        .text_xs()
                        .text_color(muted_fg)
                        .pb(px(6.0))
                        .child("确认后将提交 UPDATE 到数据库（按主键定位单行）")
                        .into_any_element()
                } else {
                    // 无主键时全列等值匹配；MySQL 拼 LIMIT 1 兜底，PG 没有 LIMIT 子句
                    // 都可能命中重复行，统一警告
                    div()
                        .text_xs()
                        .text_color(warning)
                        .pb(px(6.0))
                        .child(
                            "⚠ 该结果集无主键列：将按所有列等值匹配，\
                             如有重复行可能改到非预期那一条，请确认数据唯一性",
                        )
                        .into_any_element()
                };
                content.child(
                    div()
                        .w_full()
                        .child(hint)
                        // 显式给 Input 一个固定高度才能真正渲染成多行文本域
                        // 否则被 dialog content 的默认布局压成单行
                        .child(Input::new(&input_for_content).h(px(220.0))),
                )
            })
            .footer(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_end()
                    .gap(px(8.0))
                    .child(cancel_btn)
                    .child(apply_btn),
            )
    });
}
