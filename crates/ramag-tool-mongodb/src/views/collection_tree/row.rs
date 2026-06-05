//! 树行扁平化 + 渲染（与 dbclient::table_tree::row 同款）。所有 TreeRow 变体高度统一 28px

use gpui::{
    AnyElement, Context, IntoElement, ParentElement, SharedString, Styled, div, prelude::*, px,
};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable as _, h_flex};

use super::{CollectionTreePanel, is_system_db};

#[derive(Clone)]
pub(super) enum TreeRow {
    Database {
        name: String,
        is_expanded: bool,
    },
    /// db 展开后的占位行：loading / error
    DbPlaceholder {
        text: String,
        is_error: bool,
    },
    Collection {
        db: String,
        name: String,
        is_view: bool,
        is_selected: bool,
    },
    /// 全局占位：加载 / 错误 / 空
    GlobalPlaceholder {
        text: String,
        is_error: bool,
    },
}

impl CollectionTreePanel {
    /// 单行渲染（在 uniform_list 闭包内被调）；与 dbclient::table_tree::row 同款 28px 固定高度
    pub(super) fn render_tree_row(&self, row: &TreeRow, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted_fg = theme.muted_foreground;
        let accent = theme.accent;
        let accent_fg = theme.accent_foreground;
        let muted_bg = theme.muted;
        let danger = theme.danger;

        match row {
            TreeRow::Database { name, is_expanded } => {
                let arrow = if *is_expanded { "▾" } else { "▸" };
                let name_for_click = name.clone();
                h_flex()
                    .id(SharedString::from(format!("mongo-db-row-{name}")))
                    .h(px(28.0))
                    .flex_none()
                    .items_center()
                    .gap_1p5()
                    .px_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(move |s| s.bg(muted_bg))
                    .child(
                        div()
                            .w(px(12.0))
                            .text_xs()
                            .text_color(muted_fg)
                            .child(SharedString::from(arrow.to_string())),
                    )
                    .child(Icon::new(IconName::HardDrive).small().text_color(muted_fg))
                    .child(
                        div()
                            .text_sm()
                            .text_color(fg)
                            .whitespace_nowrap()
                            .child(SharedString::from(name.clone())),
                    )
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.toggle_database(&name_for_click, cx)
                        }),
                    )
                    .into_any_element()
            }
            TreeRow::DbPlaceholder { text, is_error } => div()
                .h(px(28.0))
                .flex_none()
                .flex()
                .items_center()
                .px(px(28.0))
                .text_xs()
                .text_color(if *is_error { danger } else { muted_fg })
                .child(SharedString::from(text.clone()))
                .into_any_element(),
            TreeRow::Collection {
                db,
                name,
                is_view,
                is_selected,
            } => {
                let db_for_click = db.clone();
                let name_for_click = name.clone();
                let selected = *is_selected;
                let mut row = h_flex()
                    .id(SharedString::from(format!("mongo-coll-row-{db}-{name}")))
                    .h(px(28.0))
                    .flex_none()
                    .items_center()
                    .gap_1()
                    .pl(px(40.0))
                    .pr_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(move |s| s.bg(muted_bg))
                    .child(
                        Icon::new(if *is_view {
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
                            .text_color(if selected { accent_fg } else { fg })
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .child(SharedString::from(name.clone())),
                    )
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.select_collection(db_for_click.clone(), name_for_click.clone(), cx)
                        }),
                    );
                if selected {
                    row = row.bg(accent);
                }
                row.into_any_element()
            }
            TreeRow::GlobalPlaceholder { text, is_error } => div()
                .h(px(28.0))
                .flex_none()
                .flex()
                .items_center()
                .px(px(12.0))
                .text_xs()
                .text_color(if *is_error { danger } else { muted_fg })
                .child(SharedString::from(text.clone()))
                .into_any_element(),
        }
    }

    /// 扁平化所有可见行（系统库过滤 + 搜索过滤）→ uniform_list 用
    pub(super) fn build_tree_rows(&self, filter: &str) -> Vec<TreeRow> {
        let mut rows: Vec<TreeRow> = Vec::with_capacity(self.databases.len() * 2);
        if self.loading && self.databases.is_empty() {
            rows.push(TreeRow::GlobalPlaceholder {
                text: "加载中…".to_string(),
                is_error: false,
            });
        }
        if let Some(err) = &self.error {
            rows.push(TreeRow::GlobalPlaceholder {
                text: err.clone(),
                is_error: true,
            });
        }
        if !self.loading && self.databases.is_empty() && self.error.is_none() {
            rows.push(TreeRow::GlobalPlaceholder {
                text: "（无数据库）".to_string(),
                is_error: false,
            });
        }

        for db in &self.databases {
            let name = &db.name;
            let name_lc = name.to_ascii_lowercase();
            if !self.show_system && is_system_db(name) {
                continue;
            }
            let exp_state = self.expanded.get(name);
            let is_expanded = exp_state.is_some();

            let coll_match = exp_state
                .map(|s| {
                    s.collections
                        .iter()
                        .any(|c| c.name.to_ascii_lowercase().contains(filter))
                })
                .unwrap_or(false);
            if !filter.is_empty() && !name_lc.contains(filter) && !coll_match {
                continue;
            }

            rows.push(TreeRow::Database {
                name: name.clone(),
                is_expanded,
            });

            if let Some(state) = exp_state {
                if state.loading {
                    rows.push(TreeRow::DbPlaceholder {
                        text: "加载中…".to_string(),
                        is_error: false,
                    });
                }
                if let Some(err) = &state.error {
                    rows.push(TreeRow::DbPlaceholder {
                        text: err.clone(),
                        is_error: true,
                    });
                }
                for c in &state.collections {
                    if !filter.is_empty()
                        && !c.name.to_ascii_lowercase().contains(filter)
                        && !name_lc.contains(filter)
                    {
                        continue;
                    }
                    let selected = self
                        .selected
                        .as_ref()
                        .is_some_and(|(d, cc)| d == name && cc == &c.name);
                    rows.push(TreeRow::Collection {
                        db: name.clone(),
                        name: c.name.clone(),
                        is_view: c.is_view,
                        is_selected: selected,
                    });
                }
            }
        }
        rows
    }
}
