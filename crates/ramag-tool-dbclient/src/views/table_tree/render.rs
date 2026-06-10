//! TableTreePanel Render：DB picker + 搜索 + 工具按钮 + uniform_list 行级虚拟化 + status bar

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    ClickEvent, Context, IntoElement, ParentElement, Render, Styled, Window, div, px, uniform_list,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Selectable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use ramag_domain::entities::{DriverKind, Schema};

use super::row::TreeRow;
use super::{TableTreePanel, TreeEvent};
use crate::sql_completion::is_system_schema;

impl Render for TableTreePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 右键操作（清空/删除）异步完成的 toast 在这里推送
        if let Some(n) = self.pending_notification.take() {
            window.push_notification(n, cx);
        }
        let muted_fg = cx.theme().muted_foreground;
        let red = gpui::red();

        // 早期返回
        if self.connection.is_none() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(muted_fg)
                .text_xs()
                .child("从左侧选一个连接")
                .into_any_element();
        }

        if self.loading_schemas {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(muted_fg)
                .text_xs()
                .child("加载 schemas...")
                .into_any_element();
        }

        if let Some(err) = self.error.clone() {
            return v_flex()
                .size_full()
                .p_2()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(red)
                        .child(format!("加载失败：{err}")),
                )
                .child(
                    Button::new("retry")
                        .small()
                        .label("重试")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.load_schemas(cx);
                        })),
                )
                .into_any_element();
        }

        // 快照状态：按 show_system + 搜索过滤
        let show_system = self.show_system;
        let filter = self.current_filter(cx);
        let has_filter = !filter.is_empty();

        let mut schemas: Vec<Schema> = self
            .schemas
            .iter()
            .filter(|s| show_system || !is_system_schema(&s.name))
            .filter(|s| {
                if !has_filter {
                    return true;
                }
                if s.name.to_ascii_lowercase().contains(&filter) {
                    return true;
                }
                if let Some(entry) = self.expanded.get(&s.name)
                    && entry
                        .tables
                        .iter()
                        .any(|t| t.name.to_ascii_lowercase().contains(&filter))
                {
                    return true;
                }
                false
            })
            .cloned()
            .collect();
        schemas.sort_by(|a, b| {
            let a_sys = is_system_schema(&a.name);
            let b_sys = is_system_schema(&b.name);
            a_sys.cmp(&b_sys).then_with(|| a.name.cmp(&b.name))
        });
        let expanded_snapshot: HashMap<
            String,
            (bool, Vec<ramag_domain::entities::Table>, Option<String>),
        > = self
            .expanded
            .iter()
            .map(|(k, v)| (k.clone(), (v.loading, v.tables.clone(), v.error.clone())))
            .collect();
        let selected = self.selected.clone();

        let mut tree_rows: Vec<TreeRow> = Vec::with_capacity(schemas.len() * 4);
        let total_schemas = self.schemas.len();
        let visible_schemas = schemas.len();
        let header_text = if total_schemas == visible_schemas {
            format!("数据库 ({total_schemas})")
        } else {
            format!("数据库 ({visible_schemas}/{total_schemas})")
        };
        let toggle_icon = if show_system {
            IconName::Eye
        } else {
            IconName::EyeOff
        };
        let toggle_tip = if show_system {
            "隐藏系统库（mysql / information_schema 等）"
        } else {
            "显示系统库（mysql / information_schema 等）"
        };
        let qp_visible = self.editor_visible;
        let qp_tip = if qp_visible {
            "隐藏 SQL 编辑器 (⌘E)"
        } else {
            "显示 SQL 编辑器 (⌘E)"
        };
        // 顶部第 1 行：schema picker（与 Redis 的 DB picker 对齐布局）
        // PG：picker 显示 `database / schema`
        let driver = self.connection.as_ref().map(|c| c.driver);
        let pg_database: Option<String> = self
            .connection
            .as_ref()
            .filter(|c| matches!(c.driver, DriverKind::Postgres))
            .and_then(|c| c.database.clone());
        let active_label = self
            .active_schema
            .clone()
            .unwrap_or_else(|| "未选库".to_string());
        let picker_label = match (driver, pg_database.as_deref()) {
            (Some(DriverKind::Postgres), Some(db)) => {
                format!("DB {db} / {active_label} ▾")
            }
            _ => format!("DB {active_label} ▾"),
        };
        let entity_for_picker = cx.entity().clone();
        let picker_schemas: Vec<String> = self
            .schemas
            .iter()
            .filter(|s| show_system || !is_system_schema(&s.name))
            .map(|s| s.name.clone())
            .collect();
        let active_for_menu = self.active_schema.clone();

        let db_row = h_flex()
            .w_full()
            .px(px(10.0))
            .py(px(6.0))
            .border_b_1()
            .border_color(cx.theme().border)
            .gap(px(8.0))
            .items_center()
            .child(
                Button::new("schema-picker")
                    .ghost()
                    .small()
                    .label(picker_label)
                    .dropdown_menu_with_anchor(gpui::Anchor::BottomLeft, move |menu, _, _| {
                        let mut m = menu;
                        let entity = entity_for_picker.clone();
                        let active = active_for_menu.clone();
                        for s in &picker_schemas {
                            let s_owned = s.clone();
                            let is_active = active.as_deref() == Some(s.as_str());
                            let label = if is_active {
                                format!("✓ {s}")
                            } else {
                                format!("  {s}")
                            };
                            let entity = entity.clone();
                            m = m.item(PopupMenuItem::new(label).on_click(move |_, _, app| {
                                let s = s_owned.clone();
                                entity.update(app, |this, cx| {
                                    if this.active_schema.as_deref() != Some(s.as_str()) {
                                        this.active_schema = Some(s.clone());
                                        cx.emit(TreeEvent::SchemaActivated { schema: s });
                                        cx.notify();
                                    }
                                });
                            }));
                        }
                        m
                    }),
            );

        // 顶部第 2 行：搜索框 + 三个工具按钮
        let header_bar = h_flex()
            .w_full()
            .items_center()
            .px(px(10.0))
            .py(px(6.0))
            .border_b_1()
            .border_color(cx.theme().border)
            .gap(px(6.0))
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.search)
                        .small()
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).small().text_color(muted_fg)),
                ),
            )
            .child(
                Button::new("toggle-system")
                    .ghost()
                    .xsmall()
                    .icon(toggle_icon)
                    .tooltip(toggle_tip)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.toggle_show_system(cx);
                    })),
            )
            .child(
                Button::new("refresh-schemas")
                    .ghost()
                    .xsmall()
                    .icon(ramag_ui::icons::refresh_cw())
                    .tooltip("刷新")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.refresh(cx);
                    })),
            )
            .child(
                Button::new("toggle-query-panel")
                    .ghost()
                    .xsmall()
                    .icon(IconName::SquareTerminal)
                    .selected(qp_visible)
                    .tooltip(qp_tip)
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        cx.emit(TreeEvent::ToggleSqlEditor);
                    })),
            );

        for s in schemas {
            let name = s.name.clone();
            let exp = expanded_snapshot.get(&name);
            let is_expanded = exp.is_some();
            let is_sys = is_system_schema(&name);

            tree_rows.push(TreeRow::Schema {
                name: name.clone(),
                is_expanded,
                is_system: is_sys,
            });

            // 展开内容
            if let Some((loading, tables, error)) = exp {
                if *loading {
                    tree_rows.push(TreeRow::SchemaPlaceholder {
                        text: "加载 tables...".into(),
                        is_error: false,
                    });
                } else if let Some(e) = error.clone() {
                    tree_rows.push(TreeRow::SchemaPlaceholder {
                        text: e,
                        is_error: true,
                    });
                } else if tables.is_empty() {
                    tree_rows.push(TreeRow::SchemaPlaceholder {
                        text: "（空）".into(),
                        is_error: false,
                    });
                } else {
                    // 按 TABLE_TYPE 分组渲染：基础表在前、视图在后
                    let total_tables = tables.iter().filter(|t| !t.is_view).count();
                    let total_views = tables.iter().filter(|t| t.is_view).count();
                    let show_group_header = total_tables > 0 && total_views > 0;
                    let mut last_was_view: Option<bool> = None;
                    for t in tables.iter() {
                        if has_filter
                            && !name.to_ascii_lowercase().contains(&filter)
                            && !t.name.to_ascii_lowercase().contains(&filter)
                        {
                            continue;
                        }
                        if show_group_header && last_was_view != Some(t.is_view) {
                            let label = if t.is_view {
                                format!("视图 ({total_views})")
                            } else {
                                format!("表 ({total_tables})")
                            };
                            tree_rows.push(TreeRow::GroupHeader { text: label });
                            last_was_view = Some(t.is_view);
                        }
                        let cols_key = format!("{}.{}", name, t.name);
                        let cols_state = self.table_columns.get(&cols_key);
                        let is_cols_expanded = cols_state.is_some();
                        let is_sel = selected.as_ref() == Some(&(name.clone(), t.name.clone()));
                        tree_rows.push(TreeRow::Table {
                            schema: name.clone(),
                            name: t.name.clone(),
                            is_view: t.is_view,
                            is_cols_expanded,
                            is_selected: is_sel,
                        });

                        if let Some(cs) = cols_state {
                            if cs.loading {
                                tree_rows.push(TreeRow::TablePlaceholder {
                                    text: "加载列结构...".into(),
                                    is_error: false,
                                });
                            } else if let Some(err) = cs.error.as_ref() {
                                tree_rows.push(TreeRow::TablePlaceholder {
                                    text: format!("加载失败：{err}"),
                                    is_error: true,
                                });
                            } else {
                                for col in cs.columns.iter() {
                                    tree_rows.push(TreeRow::Column { col: col.clone() });
                                }
                                if !cs.indexes.is_empty() {
                                    tree_rows.push(TreeRow::SectionLabel {
                                        text: format!("索引 ({})", cs.indexes.len()),
                                    });
                                    for ix in cs.indexes.iter() {
                                        let prefix = if ix.primary {
                                            "🔑 PK"
                                        } else if ix.unique {
                                            "★ UQ"
                                        } else {
                                            "·"
                                        };
                                        let line = format!(
                                            "{prefix}  {}({})",
                                            ix.name,
                                            ix.columns.join(", ")
                                        );
                                        tree_rows.push(TreeRow::DetailLine { text: line });
                                    }
                                }
                                if !cs.foreign_keys.is_empty() {
                                    tree_rows.push(TreeRow::SectionLabel {
                                        text: format!("外键 ({})", cs.foreign_keys.len()),
                                    });
                                    for fk in cs.foreign_keys.iter() {
                                        let line = format!(
                                            "↗ {} ({}) → {}.{}({})",
                                            fk.name,
                                            fk.columns.join(", "),
                                            fk.ref_schema,
                                            fk.ref_table,
                                            fk.ref_columns.join(", ")
                                        );
                                        tree_rows.push(TreeRow::DetailLine { text: line });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // uniform_list 行级虚拟化：仅渲染屏幕可见行
        let tree_rows_rc: Rc<Vec<TreeRow>> = Rc::new(tree_rows);
        let body = uniform_list(
            "mysql-tree-rows",
            tree_rows_rc.len(),
            cx.processor({
                let tree_rows_rc = tree_rows_rc.clone();
                move |this, range: Range<usize>, _w, cx| {
                    range
                        .map(|i| this.render_tree_row(&tree_rows_rc[i], cx))
                        .collect::<Vec<_>>()
                }
            }),
        )
        .track_scroll(&self.uniform_scroll)
        .flex_1();

        v_flex()
            .size_full()
            .overflow_hidden()
            .child(db_row)
            .child(header_bar)
            .child(body)
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .px_2()
                    .py(px(4.0))
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .text_color(muted_fg)
                    .child(header_text),
            )
            .into_any_element()
    }
}
