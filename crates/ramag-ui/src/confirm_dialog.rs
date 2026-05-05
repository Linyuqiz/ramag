//! 通用「破坏性操作二次确认」对话框
//!
//! 设计原则：
//! - 不绑定具体 Entity 类型；调用方在 `on_confirm` 里自己 `view.update` 自家 entity
//! - `danger=true` 用红色按钮；`false` 用 primary 蓝色（中等危险，例如 amend / merge）
//! - `on_confirm` 是 `FnOnce`，本文件用 `Rc<RefCell<Option<F>>>` 把它包成 `Fn`，符合 GPUI dialog API
//!
//! 典型用法：
//! ```ignore
//! ramag_ui::open_confirm(
//!     "删除连接？",
//!     "此操作不可恢复。",
//!     "删除",
//!     true,
//!     {
//!         let view = cx.entity();
//!         move |window, app| {
//!             view.update(app, |this, cx| this.do_delete(cx));
//!             let _ = window;
//!         }
//!     },
//!     window,
//!     cx,
//! );
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, ClickEvent, ParentElement, SharedString, Styled, Window, div, px};
use gpui_component::{
    ActiveTheme, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};

/// 弹出统一样式的二次确认对话框
///
/// 参数：
/// - `title`：对话框标题
/// - `description`：正文（多行用 `\n` 分隔）
/// - `confirm_label`：确认按钮文案，例如 `删除` / `移除` / `丢弃`
/// - `danger`：true 用红色 / false 用 primary 蓝色
/// - `on_confirm`：用户点击确认后跑（仅一次）
pub fn open_confirm(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    confirm_label: impl Into<SharedString>,
    danger: bool,
    on_confirm: impl FnOnce(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let title: SharedString = title.into();
    let description: SharedString = description.into();
    let confirm_label: SharedString = confirm_label.into();
    // FnOnce 通过 Rc<RefCell<Option<F>>> 包装成可 Clone 的 Fn 句柄
    let on_confirm_cell = Rc::new(RefCell::new(Some(on_confirm)));

    window.open_dialog(cx, move |dialog, _, _| {
        let desc = description.clone();
        let confirm_label_inner = confirm_label.clone();

        let cancel_btn = Button::new("ramag-confirm-cancel")
            .ghost()
            .small()
            .label("取消")
            .on_click(|_: &ClickEvent, window, app| {
                window.close_dialog(app);
            });

        let mut ok_btn = Button::new("ramag-confirm-ok")
            .small()
            .label(confirm_label_inner);
        ok_btn = if danger {
            ok_btn.danger()
        } else {
            ok_btn.primary()
        };

        let ok_btn = ok_btn.on_click({
            let cell = on_confirm_cell.clone();
            move |_: &ClickEvent, window, app| {
                if let Some(cb) = cell.borrow_mut().take() {
                    cb(window, app);
                }
                window.close_dialog(app);
            }
        });

        dialog
            .title(title.clone())
            .margin_top(px(180.0))
            .content(move |content, _, cx| {
                let muted_fg = cx.theme().muted_foreground;
                content.child(
                    div()
                        .py(px(4.0))
                        .text_sm()
                        .text_color(muted_fg)
                        .child(desc.clone()),
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
