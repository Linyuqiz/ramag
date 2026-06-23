//! MongoDB 主视图：左 Collection 树 + 右多 Tab 查询面板，h_resizable 中间分隔条
//!
//! 由 dbclient 在 SessionEntity::Mongo 路径下装载。事件链：
//!   tree.CollectionSelected → panel.prefill_collection（新 Tab + 自动运行 find）
//!   tree.DatabaseActivated  → panel.set_database

use std::sync::Arc;

use gpui::{
    Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window, div,
    prelude::*, px,
};
use gpui_component::{
    ActiveTheme,
    resizable::{ResizableState, h_resizable, resizable_panel},
};
use ramag_app::MongoService;
use ramag_domain::entities::ConnectionConfig;
use tracing::info;

use crate::views::collection_tree::{CollectionTreePanel, TreeEvent};
use crate::views::query_panel::MongoQueryPanel;

const TREE_WIDTH_INITIAL: f32 = 280.0;
const TREE_WIDTH_MIN: f32 = 180.0;
const TREE_WIDTH_MAX: f32 = 600.0;

pub struct MongoSessionPanel {
    config: ConnectionConfig,
    tree: Entity<CollectionTreePanel>,
    queries: Entity<MongoQueryPanel>,
    resize_state: Entity<ResizableState>,
    _subscriptions: Vec<Subscription>,
}

impl MongoSessionPanel {
    pub fn new(
        config: ConnectionConfig,
        service: Arc<MongoService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tree = cx.new(|cx| CollectionTreePanel::new(service.clone(), window, cx));
        let queries = cx.new(|cx| MongoQueryPanel::new(service.clone(), window, cx));

        // 立即同步 connection；queries 初始化一个空 Tab（与 dbclient 一致：保证至少一个 Tab）
        tree.update(cx, |t, cx| t.set_connection(Some(config.clone()), cx));
        queries.update(cx, |q, cx| {
            q.set_connection(Some(config.clone()), cx);
            q.add_tab(window, cx);
        });

        let mut subs = Vec::new();
        let queries_handle = queries.clone();
        let tree_handle = tree.clone();
        subs.push(cx.subscribe_in(
            &tree,
            window,
            move |_this: &mut Self, _, e: &TreeEvent, window, cx| match e {
                TreeEvent::CollectionSelected {
                    database,
                    collection,
                } => {
                    info!(db = %database, coll = %collection, "mongo coll selected, open tab");
                    queries_handle.update(cx, |q, cx| {
                        q.prefill_collection(database.clone(), collection.clone(), window, cx);
                    });
                }
                TreeEvent::DatabaseActivated { database } => {
                    info!(db = %database, "mongo db activated");
                    queries_handle.update(cx, |q, cx| q.set_database(database.clone(), cx));
                }
                TreeEvent::ToggleEditor => {
                    let visible = queries_handle.update(cx, |q, cx| q.toggle_editor(window, cx));
                    // 同步给 tree，让按钮图标朝向匹配
                    tree_handle.update(cx, |t, cx| t.set_editor_visible(visible, cx));
                }
            },
        ));

        let resize_state = cx.new(|_| ResizableState::default());
        Self {
            config,
            tree,
            queries,
            resize_state,
            _subscriptions: subs,
        }
    }

    pub fn config(&self) -> &ConnectionConfig {
        &self.config
    }

    pub fn title(&self) -> &str {
        &self.config.name
    }

    /// Tab 被（重新）激活时调用：collection 树为空才补拉，避免空面板（连接放久后切回也会重新请求）
    pub fn ensure_loaded(&self, cx: &mut Context<Self>) {
        self.tree.update(cx, |t, cx| t.ensure_loaded(cx));
    }
}

impl Render for MongoSessionPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div().size_full().bg(theme.background).child(
            h_resizable("mongo-session-resize")
                .with_state(&self.resize_state)
                .child(
                    resizable_panel()
                        .size(px(TREE_WIDTH_INITIAL))
                        .size_range(px(TREE_WIDTH_MIN)..px(TREE_WIDTH_MAX))
                        .child(
                            div()
                                .size_full()
                                .border_r_1()
                                .border_color(theme.border)
                                .child(self.tree.clone()),
                        ),
                )
                .child(
                    resizable_panel()
                        .child(div().size_full().min_w_0().child(self.queries.clone())),
                ),
        )
    }
}
