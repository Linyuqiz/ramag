//! 结果区顶部工具栏：过滤列 / 过滤行 / 导出 / 复制全部。
//! 行数 / 耗时摘要已下沉到底部 status bar（见 mod.rs render_status_bar），与 dbclient 一致

use gpui::{Anchor, Context, div, prelude::*, px};
use gpui_component::{
    ActiveTheme, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
};

use super::{ResultEvent, ResultPanel};

pub(super) fn render(panel: &mut ResultPanel, cx: &mut Context<ResultPanel>) -> impl IntoElement {
    let secondary = cx.theme().secondary;

    h_flex()
        .w_full()
        .flex_none()
        .px_3()
        .py(px(6.0))
        .gap_3()
        .items_center()
        .bg(secondary)
        .child(
            // 过滤列 + 过滤行：内层 flex_1 组（结构与间距均对齐 dbclient 过滤栏）
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .child({
                    // 单行 InputState 仅在多行模式注册 up/down 的 on_action，单行下补全菜单无法用方向键导航；
                    // 这里把 MoveUp/MoveDown 转发给补全菜单（与 dbclient 过滤列同款 workaround）
                    let col_for_up = panel.column_filter.clone();
                    let col_for_down = panel.column_filter.clone();
                    div()
                        .flex_1()
                        .min_w_0()
                        .on_action(move |action: &gpui_component::input::MoveUp, window, app| {
                            col_for_up.update(app, |state, cx| {
                                state.handle_action_for_context_menu(
                                    Box::new(action.clone()),
                                    window,
                                    cx,
                                );
                            });
                        })
                        .on_action(
                            move |action: &gpui_component::input::MoveDown, window, app| {
                                col_for_down.update(app, |state, cx| {
                                    state.handle_action_for_context_menu(
                                        Box::new(action.clone()),
                                        window,
                                        cx,
                                    );
                                });
                            },
                        )
                        .child(
                            Input::new(&panel.column_filter)
                                .small()
                                .bordered(false)
                                .focus_bordered(false)
                                .cleanable(true),
                        )
                })
                .child(
                    div().flex_1().min_w_0().child(
                        Input::new(&panel.row_filter)
                            .small()
                            .bordered(false)
                            .focus_bordered(false)
                            .cleanable(true),
                    ),
                ),
        )
        .child({
            let can = panel.can_write();
            Button::new("mongo-insert")
                .ghost()
                .small()
                .icon(IconName::Plus)
                .tooltip(if can {
                    "新增文档"
                } else {
                    "新增文档（需当前命令指定 collection）"
                })
                .disabled(!can || panel.is_drilled())
                .on_click(cx.listener(|panel, _, window, cx| panel.open_insert_dialog(window, cx)))
        })
        .child({
            let can_del =
                panel.can_write() && !panel.selected_rows.is_empty() && !panel.is_drilled();
            Button::new("mongo-delete")
                .ghost()
                .small()
                .icon(IconName::Minus)
                .tooltip(if can_del {
                    "删除选中文档"
                } else {
                    "删除选中文档（先勾选行）"
                })
                .disabled(!can_del)
                .on_click(cx.listener(|panel, _, window, cx| panel.open_delete_confirm(window, cx)))
        })
        .child({
            let entity = cx.entity().clone();
            let has_data = panel
                .result
                .as_ref()
                .is_some_and(|r| !r.documents.is_empty());
            Button::new("mongo-export")
                .ghost()
                .small()
                .icon(ramag_ui::icons::download())
                .tooltip("导出")
                .disabled(!has_data)
                .dropdown_menu_with_anchor(Anchor::BottomRight, move |menu, _, _| {
                    let e_json = entity.clone();
                    let e_csv = entity.clone();
                    menu.item(PopupMenuItem::new("导出 JSON").on_click(move |_, _, app| {
                        e_json.update(app, |this, cx| this.export_documents(false, cx));
                    }))
                    .item(PopupMenuItem::new("导出 CSV").on_click(move |_, _, app| {
                        e_csv.update(app, |this, cx| this.export_documents(true, cx));
                    }))
                })
        })
        .child(
            // 运行：与 dbclient 同位（结果区工具栏最右）、同图标（Play）、同快捷键（⌘↵）；
            // 点击 emit Refresh，由 query_tab 订阅后重跑当前命令
            Button::new("mongo-run-result")
                .primary()
                .small()
                .icon(IconName::Play)
                .tooltip("⌘↵ 运行")
                .on_click(cx.listener(|_panel, _, _, cx| cx.emit(ResultEvent::Refresh))),
        )
}
