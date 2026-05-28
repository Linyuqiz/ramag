//! 多 Tab 查询面板：顶部 TabBar + 当前 QueryTab。仿 dbclient::QueryPanel 行为：
//! - 横向溢出滚动（tabs_scroll 句柄 + overflow_x_scroll）
//! - 新建 Tab 自动滚到末尾
//! - cmd-w 关当前 Tab；最后一个 Tab 关闭后 propagate 给全局 fallback 关窗
//! - 由 mongo_session 在 TreeEvent::CollectionSelected 时调 prefill_collection 自动开 Tab + 运行

use std::sync::Arc;

use gpui::{
    ClickEvent, Context, Entity, IntoElement, ParentElement, Point, Render, ScrollHandle,
    SharedString, Styled, Subscription, Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use ramag_app::MongoService;
use ramag_domain::entities::ConnectionConfig;
use ramag_ui::CloseTab;

use crate::actions::{NewMongoQueryTab, ToggleMongoEditor};
use crate::views::query_tab::MongoQueryTab;

pub struct MongoQueryPanel {
    service: Arc<MongoService>,
    connection: Option<ConnectionConfig>,
    /// 当前默认 db（由 session 同步：连接配置 OR 树点击 db 行）
    database: String,
    tabs: Vec<Entity<MongoQueryTab>>,
    /// Tab 标题（与 tabs 一一对应；查询 N 自动编号，与 dbclient 一致）
    titles: Vec<String>,
    active: usize,
    /// Tab Bar 横向滚动句柄：tab 多到溢出时新建后滚到末尾
    tabs_scroll: ScrollHandle,
    /// 命令编辑器显隐（默认 false 隐藏；cmd-e 切换；新 Tab 跟随）
    show_editor: bool,
    _subscriptions: Vec<Subscription>,
}

impl MongoQueryPanel {
    pub fn new(service: Arc<MongoService>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _ = cx;
        Self {
            service,
            connection: None,
            database: "admin".to_string(),
            tabs: Vec::new(),
            titles: Vec::new(),
            active: 0,
            tabs_scroll: ScrollHandle::new(),
            // 隐藏编辑器，让结果区直接占满（与 dbclient 默认一致）
            show_editor: false,
            _subscriptions: Vec::new(),
        }
    }

    /// 切换编辑器显隐，同步给所有 tab；返回当前可见状态
    pub fn toggle_editor(&mut self, cx: &mut Context<Self>) -> bool {
        self.show_editor = !self.show_editor;
        for tab in &self.tabs {
            tab.update(cx, |t, cx| t.set_show_editor(self.show_editor, cx));
        }
        cx.notify();
        self.show_editor
    }

    pub fn set_connection(&mut self, conn: Option<ConnectionConfig>, cx: &mut Context<Self>) {
        if let Some(c) = &conn
            && let Some(db) = c.database.clone().filter(|s| !s.is_empty())
        {
            self.database = db;
        }
        self.connection = conn;
        // 重置 tabs（不同连接的 tabs 不共享上下文）
        self.tabs.clear();
        self.titles.clear();
        self.active = 0;
        cx.notify();
    }

    pub fn set_database(&mut self, db: String, cx: &mut Context<Self>) {
        if self.database != db {
            self.database = db.clone();
            for tab in &self.tabs {
                tab.update(cx, |t, cx| t.set_database(db.clone(), cx));
            }
            cx.notify();
        }
    }

    /// 树点 collection：复用当前激活 Tab（覆盖编辑器 + 运行）；如果还没 Tab 自动建一个
    pub fn prefill_collection(
        &mut self,
        database: String,
        collection: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tabs.is_empty() {
            self.add_tab(window, cx);
        }
        self.database = database.clone();
        let Some(tab) = self.tabs.get(self.active).cloned() else {
            return;
        };
        tab.update(cx, |t, cx| {
            t.prefill_for_collection(database, collection, window, cx);
            t.run(window, cx);
        });
        self.focus_active_editor(window, cx);
        cx.notify();
    }

    pub fn add_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(conf) = self.connection.clone() else {
            return;
        };
        // 找出未使用的最小编号（与 dbclient::QueryPanel 同款策略）
        let title = self.next_tab_title();
        let svc = self.service.clone();
        let db = Some(self.database.clone());
        let show_editor = self.show_editor;
        let tab = cx.new(|cx| {
            let mut t = MongoQueryTab::new(svc, conf, db, window, cx);
            t.set_show_editor(show_editor, cx);
            t
        });
        self.tabs.push(tab);
        self.titles.push(title);
        self.active = self.tabs.len() - 1;
        self.scroll_tabs_to_end();
        self.focus_active_editor(window, cx);
        cx.notify();
    }

    /// 「查询 N」自动编号：找最小未使用编号，关闭再新建会回收
    fn next_tab_title(&self) -> String {
        let mut n = 1usize;
        loop {
            let candidate = format!("查询 {n}");
            if !self.titles.iter().any(|t| t == &candidate) {
                break candidate;
            }
            n += 1;
        }
    }

    /// 聚焦当前激活 Tab 的编辑器；让 KeyContext 立即锁定到 MongoQueryTab，cmd-enter 等快捷键无需先点编辑器
    fn focus_active_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get(self.active) {
            tab.update(cx, |t, cx| t.focus_editor(window, cx));
        }
    }

    /// 大负 offset 让 tab bar 滚末尾；GPUI 自动 clamp 到 max_offset
    fn scroll_tabs_to_end(&self) {
        self.tabs_scroll
            .set_offset(Point::new(px(-99999.0), px(0.0)));
    }

    pub fn close_tab(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if idx >= self.tabs.len() {
            return;
        }
        self.tabs.remove(idx);
        if idx < self.titles.len() {
            self.titles.remove(idx);
        }
        if self.tabs.is_empty() {
            // 至少保留一个 Tab（与 dbclient 一致）
            self.add_tab(window, cx);
            return;
        }
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if self.active > idx {
            self.active -= 1;
        }
        self.focus_active_editor(window, cx);
        cx.notify();
    }

    pub fn select_tab(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if idx < self.tabs.len() && self.active != idx {
            self.active = idx;
            self.focus_active_editor(window, cx);
            cx.notify();
        }
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }
}

impl Render for MongoQueryPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;

        // Tab Bar 元素列表（一会儿放进可滚动容器）；
        // 只有 1 个 Tab 时不渲染关闭按钮（与 dbclient::QueryPanel 一致：保证至少一个 Tab）
        let only_one = self.tabs.len() <= 1;
        let tab_items: Vec<gpui::AnyElement> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, _tab)| {
                let title = self.titles.get(i).cloned().unwrap_or_default();
                let is_active = i == self.active;
                let row = h_flex()
                    .id(SharedString::from(format!("mongo-tab-{i}")))
                    .px(px(10.0))
                    .h(px(28.0))
                    .gap(px(6.0))
                    .flex_none()
                    .items_center()
                    .border_r_1()
                    .border_color(border)
                    .text_xs()
                    .when(is_active, |s| {
                        s.bg(theme.background)
                            .text_color(fg)
                            .border_b_1()
                            .border_color(theme.primary)
                    })
                    .when(!is_active, |s| s.text_color(muted))
                    .hover(|s| s.bg(theme.list_hover))
                    .cursor_pointer()
                    .child(SharedString::from(title))
                    .when(!only_one, |tab| {
                        tab.child(
                            Button::new(SharedString::from(format!("mongo-tab-close-{i}")))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    this.close_tab(i, window, cx);
                                })),
                        )
                    })
                    .on_mouse_down(
                        gpui::MouseButton::Left,
                        cx.listener(move |this, _, window, cx| this.select_tab(i, window, cx)),
                    );
                row.into_any_element()
            })
            .collect();

        // 主体：当前 Tab 内容；没 Tab 时引导提示
        let body: gpui::AnyElement = if let Some(tab) = self.tabs.get(self.active) {
            tab.clone().into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(muted)
                .text_xs()
                .child(SharedString::from(
                    "（左侧选 collection 自动开 Tab，或点 + 新 Tab）",
                ))
                .into_any_element()
        };

        v_flex()
            .size_full()
            .bg(theme.background)
            .key_context("MongoQueryPanel")
            .on_action(
                cx.listener(|this, _: &NewMongoQueryTab, window, cx| this.add_tab(window, cx)),
            )
            // 多 tab 时关当前；剩一个或没有时冒泡到全局 fallback 关窗（与 dbclient::QueryPanel 一致）
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                if this.tab_count() > 1 {
                    let i = this.active;
                    this.close_tab(i, window, cx);
                } else {
                    cx.propagate();
                }
            }))
            // cmd-e 切换编辑器显隐
            .on_action(cx.listener(|this, _: &ToggleMongoEditor, _, cx| {
                this.toggle_editor(cx);
            }))
            // Tab Bar 仅在 show_editor=true 时渲染（与 dbclient::QueryPanel 同款）
            .when(self.show_editor, |panel| {
                panel.child(
                    h_flex()
                        .w_full()
                        .flex_none()
                        .h(px(32.0))
                        .items_center()
                        .border_b_1()
                        .border_color(border)
                        .bg(theme.muted.opacity(0.10))
                        .child(
                            h_flex()
                                .id("mongo-tabs-scroll")
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .items_center()
                                .overflow_x_scroll()
                                .track_scroll(&self.tabs_scroll)
                                .children(tab_items),
                        )
                        .child(
                            h_flex()
                                .flex_none()
                                .items_center()
                                .px(px(4.0))
                                .border_l_1()
                                .border_color(border)
                                .child(
                                    Button::new("mongo-tab-add")
                                        .ghost()
                                        .small()
                                        .icon(IconName::Plus)
                                        .tooltip("新建查询")
                                        .on_click(cx.listener(
                                            |this, _: &ClickEvent, window, cx| {
                                                this.add_tab(window, cx);
                                            },
                                        )),
                                ),
                        ),
                )
            })
            .child(div().flex_1().min_h_0().child(body))
    }
}
