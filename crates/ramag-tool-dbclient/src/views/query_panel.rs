//! 多标签查询面板：顶部 TabBar + 当前 QueryTab 视图

use std::sync::Arc;

use gpui::{
    AnyView, ClickEvent, Context, Entity, InteractiveElement, IntoElement, ParentElement, Point,
    Render, ScrollHandle, SharedString, Styled, Window, div, prelude::*, px,
};

use crate::actions::NewQueryTab;
use gpui_component::{
    ActiveTheme, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use parking_lot::RwLock;
use ramag_app::ConnectionService;
use ramag_domain::entities::ConnectionConfig;
use ramag_ui::CloseTab;

use crate::sql_completion::SchemaCache;
use crate::views::query_tab::QueryTab;

pub struct QueryPanel {
    service: Arc<ConnectionService>,
    /// 共享给每个 Tab 的 SQL 补全缓存
    schema_cache: Arc<RwLock<SchemaCache>>,
    /// 各个标签页
    tabs: Vec<Entity<QueryTab>>,
    /// 标签页标题
    titles: Vec<String>,
    /// 当前激活的索引
    active: usize,
    /// 当前激活的连接（同步给所有 Tab + 历史面板）
    connection: Option<ConnectionConfig>,
    /// 当前激活的默认库（点表树/schema 行后同步给所有 Tab）
    active_schema: Option<String>,
    /// SQL 编辑器显隐（cmd-e 或表树按钮切换；全局生效，新 Tab 按此初始化）
    show_editor: bool,
    /// tab bar 横向滚动句柄：tab 多到溢出时，新建后滚到末尾让新 tab 可见
    tabs_scroll: ScrollHandle,
    _subscriptions: Vec<gpui::Subscription>,
}

impl QueryPanel {
    pub fn new(
        service: Arc<ConnectionService>,
        schema_cache: Arc<RwLock<SchemaCache>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            service,
            schema_cache,
            tabs: Vec::new(),
            titles: Vec::new(),
            active: 0,
            connection: None,
            active_schema: None,
            // 数据浏览 / 导出是主场景，写 SQL 走 cmd-e 或表树按钮唤出
            show_editor: false,
            tabs_scroll: ScrollHandle::new(),
            _subscriptions: Vec::new(),
        };
        // 默认创建一个 Tab
        this.add_tab(window, cx);
        this
    }

    /// 设置当前连接（会同步给所有 Tab + 历史面板）
    pub fn set_connection(&mut self, conn: Option<ConnectionConfig>, cx: &mut Context<Self>) {
        self.connection = conn.clone();
        // 切换连接时把 active_schema 重置为新连接的 database 字段
        self.active_schema = conn
            .as_ref()
            .and_then(|c| c.database.clone())
            .filter(|s| !s.is_empty());
        for tab in self.tabs.iter() {
            tab.update(cx, |t, cx| t.set_connection(conn.clone(), cx));
        }
        cx.notify();
    }

    /// 同步当前默认库到所有 Tab（避免 SQL 写裸表名报 No database selected）
    pub fn set_active_schema(&mut self, schema: Option<String>, cx: &mut Context<Self>) {
        let normalized = schema.filter(|s| !s.is_empty());
        if self.active_schema == normalized {
            return;
        }
        self.active_schema = normalized.clone();
        for tab in self.tabs.iter() {
            tab.update(cx, |t, cx| t.set_active_schema(normalized.clone(), cx));
        }
        cx.notify();
    }

    /// 切换 SQL 编辑器显隐：所有 Tab 同步；返回切换后的可见状态供调用方更新 UI
    pub fn toggle_editor(&mut self, cx: &mut Context<Self>) -> bool {
        self.show_editor = !self.show_editor;
        let v = self.show_editor;
        for tab in self.tabs.iter() {
            tab.update(cx, |t, cx| t.set_show_editor(v, cx));
        }
        cx.notify();
        v
    }

    fn add_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 找出未使用的最小编号（这样关闭"查询 1"再新建会重新得到"查询 1"）
        let title = {
            let mut n = 1usize;
            loop {
                let candidate = format!("查询 {n}");
                if !self.titles.iter().any(|t| t == &candidate) {
                    break candidate;
                }
                n += 1;
            }
        };
        let svc = self.service.clone();
        let conn = self.connection.clone();
        let cache = self.schema_cache.clone();
        let title_for_tab = title.clone();
        let active_schema = self.active_schema.clone();
        let initial_show_editor = self.show_editor;
        let tab = cx.new(|cx| {
            let mut t = QueryTab::new(svc, title_for_tab, conn, cache, window, cx);
            t.set_active_schema(active_schema, cx);
            // 新 Tab 跟随 panel 的全局开关初始化（隐藏态下新建 Tab 也保持隐藏）
            t.set_show_editor(initial_show_editor, cx);
            t
        });
        self.tabs.push(tab);
        self.titles.push(title);
        self.active = self.tabs.len() - 1;
        // 聚焦编辑器，cmd-t 后立即可输入
        self.focus_active_editor(window, cx);
        // 大负 offset 让 tab bar 滚末尾，GPUI 自动 clamp 到 max_offset
        self.tabs_scroll
            .set_offset(Point::new(px(-99999.0), px(0.0)));
        cx.notify();
    }

    fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        self.titles.remove(index);
        // 调整 active
        if self.tabs.is_empty() {
            self.add_tab(window, cx); // 总保持至少一个 Tab（add_tab 内部会 focus）
            return;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
        // 关闭后让新 active tab 编辑器获得焦点，无需再点一下
        self.focus_active_editor(window, cx);
        cx.notify();
    }

    fn select_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.tabs.len() && self.active != index {
            self.active = index;
            self.focus_active_editor(window, cx);
            cx.notify();
        }
    }

    /// 聚焦当前激活 Tab 的编辑器
    pub fn focus_active_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get(self.active) {
            tab.update(cx, |t, cx| t.focus_editor(window, cx));
        }
    }

    /// 把 SQL 写入当前激活 Tab 并立即执行
    pub fn prefill_active_sql_and_run(
        &mut self,
        sql: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.tabs.get(self.active) {
            tab.update(cx, |t, cx| {
                t.set_sql(sql, window, cx);
                t.run(cx);
            });
        }
    }

    /// 同 prefill_active_sql_and_run，额外注入精确目标表 (schema, table)
    /// 表树点击触发的 SELECT 用：避开反引号内带短横线被 SQL parser 吞的坑
    pub fn prefill_active_sql_and_run_with_target(
        &mut self,
        sql: String,
        target: Option<(String, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.tabs.get(self.active) {
            tab.update(cx, |t, cx| {
                // set_sql 内会清 pinned_target，所以必须先 set_sql 再 set_pinned_target
                t.set_sql(sql, window, cx);
                t.set_pinned_target(target);
                // 切表时同步清空两个过滤框，避免旧 filter 挡新表数据
                t.clear_result_filters(window, cx);
                t.run(cx);
            });
        }
    }

    /// 新建一个 Tab 写入 SQL 并立即执行（用于 SHOW CREATE TABLE 等辅助查询，
    /// 不污染用户当前编辑的 Tab）
    pub fn open_in_new_tab_and_run(
        &mut self,
        sql: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_tab(window, cx);
        self.prefill_active_sql_and_run(sql, window, cx);
    }

    /// 把示例 SQL 插入当前激活 Tab 的编辑器（Tab 栏「示例」下拉用）
    fn insert_example_into_active(
        &mut self,
        sql: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab) = self.tabs.get(self.active).cloned() {
            tab.update(cx, |t, cx| t.insert_example(sql, window, cx));
        }
    }
}

impl Render for QueryPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let border = theme.border;
        let secondary_bg = theme.secondary;
        let muted_bg = theme.muted;
        let accent = theme.accent;

        let active = self.active;
        // 优先用 QueryTab 的 display_title（执行后变 SQL 摘要），fallback 到默认 titles
        let titles: Vec<String> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let dt = t.read(cx).display_title().to_string();
                if dt.is_empty() {
                    self.titles.get(i).cloned().unwrap_or_default()
                } else {
                    dt
                }
            })
            .collect();
        let only_one = titles.len() <= 1;

        // 当前主区视图：始终是 active Tab
        let current_view: Option<AnyView> = self.tabs.get(active).map(|t| t.clone().into());

        // Tab Bar 渲染
        let tab_bar_items: Vec<gpui::AnyElement> = titles
            .iter()
            .enumerate()
            .map(|(idx, title)| {
                let is_active = idx == active;
                let title = title.clone();
                let id_select = SharedString::from(format!("tab-{idx}"));
                let id_close = SharedString::from(format!("tab-close-{idx}"));

                let mut tab = h_flex()
                    .id(id_select)
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py(px(7.0))
                    .border_r_1()
                    .border_color(border)
                    .cursor_pointer()
                    .child(
                        div()
                            .text_xs()
                            .text_color(if is_active { fg } else { muted_fg })
                            .child(title),
                    )
                    .when(!only_one, |tab| {
                        tab.child(
                            Button::new(id_close)
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    this.close_tab(idx, window, cx);
                                })),
                        )
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.select_tab(idx, window, cx);
                    }));

                if is_active {
                    tab = tab.bg(theme_active_bg(secondary_bg, accent));
                } else {
                    tab = tab.hover(move |this| this.bg(muted_bg));
                }

                tab.into_any_element()
            })
            .collect();

        v_flex()
            .size_full()
            .key_context("QueryPanel")
            // 监听 NewQueryTab / CloseTab，绑定见 main.rs
            .on_action(cx.listener(|this, _: &NewQueryTab, window, cx| {
                this.add_tab(window, cx);
            }))
            // 多 tab 时关当前；剩一个时冒泡到全局 fallback 关窗（VSCode 风）
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                if this.tabs.len() > 1 {
                    let idx = this.active;
                    this.close_tab(idx, window, cx);
                } else {
                    cx.propagate();
                }
            }))
            // Tab Bar 仅在 SQL 编辑器可见时渲染
            .when(self.show_editor, |panel| {
                panel.child(
                    h_flex()
                        .w_full()
                        .flex_none()
                        .border_b_1()
                        .border_color(border)
                        .bg(secondary_bg)
                        // 左：tabs 区，溢出时横向滚动；min_w_0 让它能被压缩
                        .child(
                            h_flex()
                                .id("query-tabs-scroll")
                                .flex_1()
                                .min_w_0()
                                .overflow_x_scroll()
                                .track_scroll(&self.tabs_scroll)
                                .children(tab_bar_items)
                                // + 新建按钮跟在最后一个 tab 之后
                                .child(
                                    Button::new("tab-add")
                                        .ghost()
                                        .small()
                                        .icon(IconName::Plus)
                                        .tooltip("新建查询 (⌘T)")
                                        .on_click(cx.listener(
                                            |this, _: &ClickEvent, window, cx| {
                                                this.add_tab(window, cx);
                                            },
                                        )),
                                ),
                        )
                        // 右：示例 / 格式化 / EXPLAIN（历史改弹框，不放这里）
                        .child(
                            h_flex()
                                .flex_none()
                                .items_center()
                                .border_l_1()
                                .border_color(border)
                                .child({
                                    let entity = cx.entity();
                                    let driver = self.connection.as_ref().map(|c| c.driver);
                                    // 模板个性化：用当前 Tab 最近点开的表名，没有则占位
                                    let table = self
                                        .tabs
                                        .get(self.active)
                                        .and_then(|t| {
                                            t.read(cx)
                                                .pinned_target
                                                .as_ref()
                                                .map(|(_, table)| table.clone())
                                        })
                                        .unwrap_or_default();
                                    Button::new("sql-examples")
                                        .ghost()
                                        .small()
                                        .icon(ramag_ui::icons::scroll_text())
                                        .tooltip("常用 SQL 示例（插入编辑器）")
                                        .disabled(driver.is_none())
                                        .dropdown_menu(move |menu, _, _| {
                                            let Some(driver) = driver else {
                                                return menu;
                                            };
                                            let mut m = menu;
                                            for (label, sql) in
                                                super::query_tab::sql_examples(driver, &table)
                                            {
                                                let e = entity.clone();
                                                m = m.item(PopupMenuItem::new(label).on_click(
                                                    move |_, window, app| {
                                                        e.update(app, |panel, cx| {
                                                            panel.insert_example_into_active(
                                                                &sql, window, cx,
                                                            );
                                                        });
                                                    },
                                                ));
                                            }
                                            m
                                        })
                                })
                                .child(
                                    Button::new("format-sql")
                                        .ghost()
                                        .small()
                                        .icon(ramag_ui::icons::wand_sparkles())
                                        .tooltip("美化 SQL (⌘⇧F)")
                                        .on_click(cx.listener(
                                            |this, _: &ClickEvent, window, cx| {
                                                if let Some(tab) =
                                                    this.tabs.get(this.active).cloned()
                                                {
                                                    tab.update(cx, |t, cx| {
                                                        t.handle_format(window, cx)
                                                    });
                                                }
                                            },
                                        )),
                                )
                                .child(
                                    Button::new("explain-sql")
                                        .ghost()
                                        .small()
                                        .icon(ramag_ui::icons::gauge())
                                        .tooltip("执行计划 EXPLAIN (⌘⇧E)")
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                            if let Some(tab) = this.tabs.get(this.active).cloned() {
                                                tab.update(cx, |t, cx| t.handle_explain(cx));
                                            }
                                        })),
                                ),
                        ),
                )
            })
            // 当前 Tab 内容
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .when_some(current_view, |this, view| this.child(view)),
            )
    }
}

/// 选中 Tab 的背景色：在 secondary 上叠加微弱 accent
fn theme_active_bg(_secondary: gpui::Hsla, accent: gpui::Hsla) -> gpui::Hsla {
    let mut a = accent;
    a.a = 0.15;
    a
}
