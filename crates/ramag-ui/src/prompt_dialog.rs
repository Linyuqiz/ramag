//! 单行文本输入对话框（与 open_confirm 对称）。用于重命名等需要输入新值的轻量操作；
//! 确认时把 trim 后的输入交给 on_confirm，空输入不触发

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    App, AppContext as _, ClickEvent, Entity, ParentElement, SharedString, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    v_flex,
};

pub fn open_prompt(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    initial: &str,
    confirm_label: impl Into<SharedString>,
    on_confirm: impl FnOnce(String, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let title: SharedString = title.into();
    let description: SharedString = description.into();
    let confirm_label: SharedString = confirm_label.into();
    let input: Entity<InputState> = cx.new(|cx| InputState::new(window, cx).default_value(initial));
    // FnOnce 包成可 Clone 的 Fn 句柄
    let on_confirm_cell = Rc::new(RefCell::new(Some(on_confirm)));

    window.open_dialog(cx, move |dialog, _, _| {
        let desc = description.clone();
        let confirm_label_inner = confirm_label.clone();

        let cancel_btn = Button::new("ramag-prompt-cancel")
            .ghost()
            .small()
            .label("取消")
            .on_click(|_: &ClickEvent, window, app| {
                window.close_dialog(app);
            });

        let ok_btn = Button::new("ramag-prompt-ok")
            .small()
            .primary()
            .label(confirm_label_inner)
            .on_click({
                let cell = on_confirm_cell.clone();
                let input = input.clone();
                move |_: &ClickEvent, window, app| {
                    let value = input.read(app).value().trim().to_string();
                    if value.is_empty() {
                        return;
                    }
                    if let Some(cb) = cell.borrow_mut().take() {
                        cb(value, window, app);
                    }
                    window.close_dialog(app);
                }
            });

        let input_for_content = input.clone();
        dialog
            .title(title.clone())
            .margin_top(px(180.0))
            .content(move |content, _, cx| {
                let muted_fg = cx.theme().muted_foreground;
                content.child(
                    v_flex()
                        .gap(px(8.0))
                        .py(px(4.0))
                        .child(div().text_sm().text_color(muted_fg).child(desc.clone()))
                        .child(Input::new(&input_for_content).small()),
                )
            })
            .footer(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_end()
                    .gap(px(8.0))
                    .child(cancel_btn)
                    .child(ok_btn),
            )
    });
}
