//! `impl Render for QueryTab`：编辑器 + 工具条 + 结果区。按钮行为在 actions

use gpui::{
    AppContext as _, ClickEvent, Context, Entity, IntoElement, ParentElement, Render, Styled,
    Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, IconName, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    menu::DropdownMenu as _,
    notification::Notification,
    v_flex,
};

use super::QueryTab;
use super::sql_utils::{AUTO_LIMIT, format_elapsed};
use crate::actions::{
    ExplainQuery, ExportCsv, ExportJson, ExportMarkdown, FormatSql, RunQuery, RunStatementAtCursor,
};
use crate::views::result_panel::ResultState;

impl Render for QueryTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let border = theme.border;
        let secondary_bg = theme.secondary;
        let bg = theme.background;

        let running = self.running;
        let has_connection = self.connection.is_some();

        // 仅"执行中"状态在工具条显示实时耗时，其他状态由结果面板底部 status_bar 展示
        let running_elapsed = self.query_start.map(|t| t.elapsed()).map(format_elapsed);
        let (result_summary, has_result): (Option<String>, bool) =
            match self.result.read(cx).state() {
                ResultState::Ok(qr) => (None, !qr.rows.is_empty()),
                ResultState::Error(_) => (None, false),
                ResultState::Running => (
                    Some(match &running_elapsed {
                        Some(s) => format!("执行中 {s}"),
                        None => "执行中".to_string(),
                    }),
                    false,
                ),
                ResultState::Empty => (None, false),
            };
        let panel_for_btn = self.result.read(cx);
        let has_multi_selected = !panel_for_btn.selected_rows().is_empty();
        let has_selected = has_multi_selected || panel_for_btn.selected_cell().is_some();
        let target_is_view = panel_for_btn.target_is_view();
        let _ = panel_for_btn;

        // 分页控件：只在"翻过页或还有下一页"时显示，单页结果不打扰
        let pager_ui: Option<(usize, bool, String)> = self.pager.as_ref().and_then(|p| {
            if p.page == 0 && !p.has_more {
                return None;
            }
            let label = match self.result.read(cx).state() {
                ResultState::Ok(qr) if !qr.rows.is_empty() => {
                    let start = p.page * AUTO_LIMIT + 1;
                    let end = p.page * AUTO_LIMIT + qr.rows.len();
                    format!("{start}–{end} 行")
                }
                _ => format!("第 {} 页", p.page + 1),
            };
            Some((p.page, p.has_more, label))
        });

        v_flex()
            .size_full()
            .bg(bg)
            .key_context("QueryTab")
            .on_action(cx.listener(|this, _: &RunQuery, _, cx| {
                this.handle_run(cx);
            }))
            .on_action(cx.listener(|this, _: &RunStatementAtCursor, _, cx| {
                this.handle_run_at_cursor(cx);
            }))
            .on_action(cx.listener(|this, _: &ExportCsv, _, cx| {
                this.result.update(cx, |r, cx| {
                    r.export(crate::views::result_panel::ExportFormat::Csv, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &ExportJson, _, cx| {
                this.result.update(cx, |r, cx| {
                    r.export(crate::views::result_panel::ExportFormat::Json, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &FormatSql, window, cx| {
                this.handle_format(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ExplainQuery, _, cx| {
                this.handle_explain(cx);
            }))
            .when(self.show_editor, |this| {
                this.child(
                    div()
                        .h(px(220.0))
                        .flex_none()
                        .border_b_1()
                        .border_color(border)
                        .child(
                            Input::new(&self.editor)
                                .h_full()
                                .bordered(false)
                                .focus_bordered(false),
                        ),
                )
            })
            .child(
                h_flex()
                    .w_full()
                    .flex_none()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py(px(6.0))
                    .border_b_1()
                    .border_color(border)
                    .bg(secondary_bg)
                    .child({
                        let col_input = self.result.read(cx).column_filter_entity().clone();
                        let row_input = self.result.read(cx).row_filter_entity().clone();
                        let col_for_up = col_input.clone();
                        let col_for_down = col_input.clone();
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .on_action(
                                        move |action: &gpui_component::input::MoveUp,
                                              window,
                                              app| {
                                            col_for_up.update(app, |state, cx| {
                                                state.handle_action_for_context_menu(
                                                    Box::new(action.clone()),
                                                    window,
                                                    cx,
                                                );
                                            });
                                        },
                                    )
                                    .on_action(
                                        move |action: &gpui_component::input::MoveDown,
                                              window,
                                              app| {
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
                                        Input::new(&col_input)
                                            .small()
                                            .bordered(false)
                                            .focus_bordered(false)
                                            .cleanable(true),
                                    ),
                            )
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&row_input)
                                        .small()
                                        .bordered(false)
                                        .focus_bordered(false)
                                        .cleanable(true),
                                ),
                            )
                    })
                    .when_some(result_summary, |this, summary| {
                        this.child(div().text_xs().text_color(muted_fg).child(summary))
                    })
                    .when_some(pager_ui, |this, (page, has_more, label)| {
                        this.child(
                            h_flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    Button::new("pager-prev")
                                        .ghost()
                                        .small()
                                        .icon(IconName::ChevronLeft)
                                        .tooltip("上一页")
                                        .disabled(page == 0 || running)
                                        .on_click(cx.listener(
                                            move |this, _: &ClickEvent, _, cx| {
                                                this.handle_page(page.saturating_sub(1), cx);
                                            },
                                        )),
                                )
                                .child(div().text_xs().text_color(muted_fg).child(label))
                                .child(
                                    Button::new("pager-next")
                                        .ghost()
                                        .small()
                                        .icon(IconName::ChevronRight)
                                        .tooltip("下一页")
                                        .disabled(!has_more || running)
                                        .on_click(cx.listener(
                                            move |this, _: &ClickEvent, _, cx| {
                                                this.handle_page(page + 1, cx);
                                            },
                                        )),
                                ),
                        )
                    })
                    .child({
                        let can_insert = self.connection.is_some()
                            && self.pinned_target.is_some()
                            && !target_is_view
                            && self.result.read(cx).pending_insert().is_none();
                        Button::new("toolbar-insert")
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .tooltip(if can_insert {
                                "新增行"
                            } else if target_is_view {
                                "新增行（视图不可写入）"
                            } else if self.pinned_target.is_none() {
                                "新增行（请先从表树点开单表）"
                            } else {
                                "新增行（已在草稿中，先提交或取消）"
                            })
                            .disabled(!can_insert)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                let Some(conn) = this.connection.clone() else {
                                    return;
                                };
                                let Some((schema, table)) = this.pinned_target.clone() else {
                                    return;
                                };
                                let svc = this.service.clone();
                                let panel = this.result.clone();
                                let handle = window.window_handle();
                                cx.spawn(async move |_, cx| {
                                    let cols = svc.list_columns(&conn, &schema, &table).await;
                                    let _ = cx.update_window(handle, |_, window, app| match cols {
                                        Ok(cols) => {
                                            let inputs: Vec<Entity<InputState>> = cols
                                                .iter()
                                                .map(|col| {
                                                    let placeholder = format!(
                                                        "{} · {}",
                                                        col.data_type.raw_type,
                                                        if col.nullable {
                                                            "可空"
                                                        } else {
                                                            "必填"
                                                        }
                                                    );
                                                    app.new(|cx_inner| {
                                                        InputState::new(window, cx_inner)
                                                            .placeholder(placeholder)
                                                    })
                                                })
                                                .collect();
                                            let first_input = inputs.first().cloned();
                                            panel.update(app, |r, cx| {
                                                r.start_insert(cols, inputs, cx);
                                            });
                                            if let Some(input) = first_input {
                                                input.update(app, |state, cx_inner| {
                                                    state.focus(window, cx_inner);
                                                });
                                            }
                                        }
                                        Err(e) => {
                                            window.push_notification(
                                                Notification::error(format!("拉取表结构失败：{e}"))
                                                    .autohide(true),
                                                app,
                                            );
                                        }
                                    });
                                })
                                .detach();
                            }))
                    })
                    .child(
                        Button::new("toolbar-delete")
                            .ghost()
                            .small()
                            .icon(IconName::Minus)
                            .tooltip(if target_is_view {
                                "删除选中行（视图不可写入）"
                            } else {
                                "删除选中行"
                            })
                            .disabled(!has_selected || target_is_view)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                let panel_ref = this.result.read(cx);
                                let multi = panel_ref.delete_preview_multi();
                                let single = if multi.is_none() {
                                    panel_ref.delete_preview()
                                } else {
                                    None
                                };
                                let _ = panel_ref;
                                let result = this.result.clone();
                                let (title, preview, on_ok_indices, on_ok_single): (
                                    &'static str,
                                    String,
                                    Option<Vec<usize>>,
                                    Option<usize>,
                                ) = match (multi, single) {
                                    (Some((ids, summary)), _) => {
                                        ("删除选中行？", summary, Some(ids), None)
                                    }
                                    (None, Some((ri, p))) => {
                                        ("删除此行？", format!("将删除：{p}"), None, Some(ri))
                                    }
                                    _ => return,
                                };
                                window.open_dialog(cx, move |dialog, _, _| {
                                    let result_btn = result.clone();
                                    let preview_for_content = preview.clone();
                                    let on_ok_indices = on_ok_indices.clone();
                                    let on_ok_single = on_ok_single;
                                    let cancel = Button::new("del-row-cancel")
                                        .ghost()
                                        .small()
                                        .label("取消")
                                        .on_click(|_: &ClickEvent, window, app| {
                                            window.close_dialog(app);
                                        });
                                    let ok = Button::new("del-row-ok")
                                        .danger()
                                        .small()
                                        .label("删除")
                                        .on_click({
                                            let result = result_btn.clone();
                                            let indices = on_ok_indices.clone();
                                            let single = on_ok_single;
                                            move |_: &ClickEvent, window, app| {
                                                result.update(app, |r, cx| {
                                                    if let Some(ids) = indices.clone() {
                                                        r.execute_delete_rows_async(ids, cx);
                                                    } else if let Some(ri) = single {
                                                        r.execute_delete_row_async(ri, cx);
                                                    }
                                                });
                                                window.close_dialog(app);
                                            }
                                        });
                                    dialog
                                        .title(title)
                                        .width(px(520.0))
                                        .margin_top(px(180.0))
                                        .content(move |c, _, cx| {
                                            let muted_fg = cx.theme().muted_foreground;
                                            let p = preview_for_content.clone();
                                            c.child(div().text_sm().text_color(muted_fg).child(p))
                                        })
                                        .footer(
                                            h_flex()
                                                .w_full()
                                                .items_center()
                                                .justify_end()
                                                .gap(px(8.0))
                                                .child(cancel)
                                                .child(ok),
                                        )
                                });
                            })),
                    )
                    .child(
                        Button::new("export-btn")
                            .ghost()
                            .small()
                            .icon(ramag_ui::icons::download())
                            .tooltip("导出")
                            .disabled(!has_result)
                            .dropdown_menu(|menu, _, _| {
                                menu.menu("CSV", Box::new(ExportCsv))
                                    .menu("JSON", Box::new(ExportJson))
                                    .menu("Markdown", Box::new(ExportMarkdown))
                            }),
                    )
                    .when(running, |this| {
                        this.child(
                            Button::new("cancel-query")
                                .danger()
                                .small()
                                .icon(IconName::Close)
                                .tooltip("取消当前查询")
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.handle_cancel(window, cx);
                                })),
                        )
                    })
                    .when(!running, |this| {
                        this.child(
                            Button::new("run-query")
                                .primary()
                                .small()
                                .icon(IconName::Play)
                                .disabled(!has_connection)
                                .tooltip("⌘↵ 运行 SQL")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.handle_run(cx);
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(self.result.clone()),
            )
    }
}
