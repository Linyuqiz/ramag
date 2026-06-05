//! 扁平化树行 + 渲染。所有 TreeRow 变体高度统一 28px（uniform_list 硬约束）

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem},
};
use ramag_domain::entities::Column;

use super::TableTreePanel;
use crate::views::tree_helpers::{render_column_row, render_columns_placeholder};

#[derive(Clone)]
pub(super) enum TreeRow {
    /// schema 行：可点击展开/折叠
    Schema {
        name: String,
        is_expanded: bool,
        is_system: bool,
    },
    /// schema 下的占位行：loading / error / 空
    SchemaPlaceholder { text: String, is_error: bool },
    /// 分组小标题："表 (N)" / "视图 (N)"
    GroupHeader { text: String },
    /// 表/视图行
    Table {
        schema: String,
        name: String,
        is_view: bool,
        is_cols_expanded: bool,
        is_selected: bool,
    },
    /// 表的列结构占位行：loading / error
    TablePlaceholder { text: String, is_error: bool },
    /// 列定义行
    Column { col: Column },
    /// "索引 (N)" / "外键 (N)" 小标题
    SectionLabel { text: String },
    /// 索引 / 外键 的详情行
    DetailLine { text: String },
}

impl TableTreePanel {
    /// 渲染单条 TreeRow（在 uniform_list 闭包内被调）
    pub(super) fn render_tree_row(&self, row: &TreeRow, cx: &mut Context<Self>) -> AnyElement {
        let muted_fg = cx.theme().muted_foreground;
        let muted_bg = cx.theme().muted;
        let accent_bg = cx.theme().accent;
        let accent_fg = cx.theme().accent_foreground;
        let fg = cx.theme().foreground;
        let red = gpui::red();

        match row {
            TreeRow::Schema {
                name,
                is_expanded,
                is_system,
            } => {
                let arrow = if *is_expanded { "▾" } else { "▸" };
                let id_str = SharedString::from(format!("schema-{name}"));
                let name_for_click = name.clone();
                let name_color = if *is_system { muted_fg } else { fg };

                h_flex()
                    .id(id_str)
                    .h(px(28.0))
                    .flex_none()
                    .items_center()
                    .gap_1p5()
                    .px_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(move |this| this.bg(muted_bg))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.toggle_schema(name_for_click.clone(), cx);
                    }))
                    .child(
                        div()
                            .w(px(12.0))
                            .text_xs()
                            .text_color(muted_fg)
                            .child(arrow),
                    )
                    .child(Icon::new(IconName::HardDrive).small().text_color(muted_fg))
                    .child(
                        div()
                            .text_sm()
                            .text_color(name_color)
                            .whitespace_nowrap()
                            .child(name.clone()),
                    )
                    .into_any_element()
            }
            TreeRow::SchemaPlaceholder { text, is_error } => div()
                .w_full()
                .h(px(28.0))
                .flex_none()
                .pl_5()
                .pr_2()
                .pt(px(6.0))
                .text_xs()
                .text_color(if *is_error { red } else { muted_fg })
                .whitespace_nowrap()
                .overflow_hidden()
                .text_ellipsis()
                .child(text.clone())
                .into_any_element(),
            TreeRow::GroupHeader { text } => div()
                .w_full()
                .h(px(28.0))
                .flex_none()
                .pl_5()
                .pr_2()
                .pt(px(6.0))
                .text_xs()
                .text_color(muted_fg)
                .child(text.clone())
                .into_any_element(),
            TreeRow::Table {
                schema,
                name,
                is_view,
                is_cols_expanded,
                is_selected,
            } => {
                let schema = schema.clone();
                let name = name.clone();
                let is_view = *is_view;
                let is_cols_expanded = *is_cols_expanded;
                let is_selected = *is_selected;

                let row_id = SharedString::from(format!("table-{}-{}", schema, name));
                let s_for_click = schema.clone();
                let t_for_click = name.clone();

                let chevron_icon = if is_cols_expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                };
                let chevron_id = SharedString::from(format!("col-toggle-{}-{}", schema, name));
                let s_for_chev = schema.clone();
                let t_for_chev = name.clone();
                let s_for_menu = schema.clone();
                let t_for_menu = name.clone();
                let entity_for_menu = cx.entity().clone();

                let mut row = h_flex()
                    .id(row_id)
                    .h(px(28.0))
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .pl(px(20.0))
                    .pr_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(move |this| this.bg(muted_bg))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.handle_table_click(s_for_click.clone(), t_for_click.clone(), cx);
                    }))
                    // chevron 单击只展开列结构，不触发 TableSelected
                    .child(
                        div()
                            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation()
                            })
                            .child(
                                Button::new(chevron_id)
                                    .ghost()
                                    .xsmall()
                                    .icon(chevron_icon)
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.toggle_table_columns(
                                            s_for_chev.clone(),
                                            t_for_chev.clone(),
                                            cx,
                                        );
                                    })),
                            ),
                    )
                    .child(
                        Icon::new(if is_view {
                            IconName::Frame
                        } else {
                            IconName::MemoryStick
                        })
                        .small()
                        .text_color(muted_fg),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if is_selected { accent_fg } else { fg })
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(name.clone()),
                    );
                if is_selected {
                    row = row.bg(accent_bg);
                }
                let menu_label = if is_view {
                    "查看视图定义"
                } else {
                    "查看建表 SQL"
                };
                let row = row.context_menu(move |menu: PopupMenu, _, _| {
                    let s = s_for_menu.clone();
                    let t = t_for_menu.clone();
                    let ent = entity_for_menu.clone();
                    menu.item(PopupMenuItem::new(menu_label).on_click(move |_e, _w, app| {
                        let s = s.clone();
                        let t = t.clone();
                        ent.update(app, |this, cx| {
                            this.handle_show_ddl(s, t, is_view, cx);
                        });
                    }))
                });
                row.into_any_element()
            }
            TreeRow::TablePlaceholder { text, is_error } => {
                render_columns_placeholder(text.clone(), if *is_error { red } else { muted_fg })
            }
            TreeRow::Column { col } => render_column_row(col, fg, muted_fg),
            TreeRow::SectionLabel { text } => render_columns_placeholder(text.clone(), muted_fg),
            TreeRow::DetailLine { text } => render_columns_placeholder(text.clone(), fg),
        }
    }
}
