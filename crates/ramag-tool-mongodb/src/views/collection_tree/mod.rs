//! Database → Collection 双层树。点 collection 触发查询 Tab 自动 find({})

mod ops;
mod row;

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    Anchor, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, SharedString,
    Styled, Subscription, UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};

use gpui_component::{
    ActiveTheme, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use ramag_app::MongoService;
use ramag_domain::entities::{ConnectionConfig, MongoCollection, MongoDatabase};
use row::TreeRow;
use tracing::{error, info};

pub struct CollectionTreePanel {
    service: Arc<MongoService>,
    connection: Option<ConnectionConfig>,
    /// 已加载的 database 列表
    databases: Vec<MongoDatabase>,
    /// 加载中标记
    loading: bool,
    error: Option<String>,
    /// 已展开的 db → collection 列表（None=未展开，Some(vec)=已加载）
    expanded: HashMap<String, ExpandedState>,
    /// 当前选中（database, collection）
    selected: Option<(String, String)>,
    /// 当前激活的 database（用户点 db 行或 collection 行时更新；顶部 header 显示）
    active_db: Option<String>,
    /// 搜索框（按名字模糊过滤 db / collection）
    search: Entity<InputState>,
    /// 是否显示系统库（admin / config / local）。默认隐藏
    show_system: bool,
    /// 父级（query_panel）当前命令编辑器是否可见。仅用于按钮图标朝向；点按后 emit 给父级
    editor_visible: bool,
    /// 树体行虚拟化滚动句柄（与 dbclient::table_tree 同款）
    uniform_scroll: UniformListScrollHandle,
    /// 切连接后是否待自动展开默认库（仅首次加载消费一次，refresh 不重复展开）
    auto_expand_pending: bool,
    /// 右键操作（清空/删除）完成后的 toast，下次 render 推送
    pending_notification: Option<gpui_component::notification::Notification>,
    _subscriptions: Vec<Subscription>,
}

/// MongoDB 系统库名（与 MySQL information_schema / mysql 等同位）
const SYSTEM_DBS: &[&str] = &["admin", "config", "local"];

pub(super) fn is_system_db(name: &str) -> bool {
    SYSTEM_DBS.contains(&name)
}

/// 选默认展开的 db：连接配置的 database 优先（须在列表内），否则首个非系统库
fn pick_default_db(conn: Option<&ConnectionConfig>, databases: &[MongoDatabase]) -> Option<String> {
    if let Some(db) = conn
        .and_then(|c| c.database.as_deref())
        .filter(|s| !s.is_empty())
        && databases.iter().any(|d| d.name == db)
    {
        return Some(db.to_string());
    }
    databases
        .iter()
        .find(|d| !is_system_db(&d.name))
        .map(|d| d.name.clone())
}

#[derive(Default)]
struct ExpandedState {
    loading: bool,
    collections: Vec<MongoCollection>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum TreeEvent {
    /// 用户点了 collection：父级在新 Tab 自动 `find({}).limit(10000)`（与 dbclient AUTO_LIMIT 同款）
    CollectionSelected {
        database: String,
        collection: String,
    },
    /// 用户点了 database 行，切换"当前 db"
    DatabaseActivated { database: String },
    /// 用户点了"切换命令编辑器"按钮，父级（query_panel）执行 toggle_editor 并把新状态回填给 tree
    ToggleEditor,
}

impl EventEmitter<TreeEvent> for CollectionTreePanel {}

impl CollectionTreePanel {
    pub fn new(service: Arc<MongoService>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("搜索 database / collection")
                .clean_on_escape()
        });
        let subs = vec![cx.subscribe(&search, |_this, _, _e: &InputEvent, cx| cx.notify())];
        Self {
            service,
            connection: None,
            databases: Vec::new(),
            loading: false,
            error: None,
            expanded: HashMap::new(),
            selected: None,
            active_db: None,
            search,
            show_system: false,
            editor_visible: false,
            uniform_scroll: UniformListScrollHandle::new(),
            auto_expand_pending: false,
            pending_notification: None,
            _subscriptions: subs,
        }
    }

    fn toggle_show_system(&mut self, cx: &mut Context<Self>) {
        self.show_system = !self.show_system;
        cx.notify();
    }

    /// 父级（query_panel）切完编辑器后回填新可见态，让按钮图标朝向匹配
    pub fn set_editor_visible(&mut self, v: bool, cx: &mut Context<Self>) {
        if self.editor_visible != v {
            self.editor_visible = v;
            cx.notify();
        }
    }

    /// 连接切换：清空旧状态，异步拉 db 列表。如果连接配置带 database 字段，预填到 active_db
    pub fn set_connection(&mut self, conn: Option<ConnectionConfig>, cx: &mut Context<Self>) {
        self.active_db = conn
            .as_ref()
            .and_then(|c| c.database.clone())
            .filter(|s| !s.is_empty());
        self.connection = conn;
        self.databases.clear();
        self.expanded.clear();
        self.selected = None;
        self.error = None;
        // 切连接后首次加载完 db 列表时自动展开默认库（仅一次）
        self.auto_expand_pending = self.connection.is_some();
        if self.connection.is_some() {
            self.refresh_databases(cx);
        }
        cx.notify();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.refresh_databases(cx);
        // 同时强制重拉所有已展开 db 的 collection 列表，否则新建的 collection 不会出现
        let expanded_dbs: Vec<String> = self.expanded.keys().cloned().collect();
        for db in expanded_dbs {
            self.load_collections(db, cx);
        }
    }

    fn refresh_databases(&mut self, cx: &mut Context<Self>) {
        let Some(conf) = self.connection.clone() else {
            return;
        };
        let svc = self.service.clone();
        self.loading = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = svc.list_databases(&conf).await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match r {
                    Ok(dbs) => {
                        info!(count = dbs.len(), "mongo databases loaded");
                        this.databases = dbs;
                        // 首次加载：自动展开并激活默认库（config.database 优先，否则首个非系统库）
                        if this.auto_expand_pending {
                            this.auto_expand_pending = false;
                            if let Some(default_db) =
                                pick_default_db(this.connection.as_ref(), &this.databases)
                            {
                                this.toggle_database(&default_db, cx);
                            }
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "mongo list_databases failed");
                        this.error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_database(&mut self, db: &str, cx: &mut Context<Self>) {
        // 同时记录"当前激活 db"用于顶部展示，并 emit 给查询面板
        self.active_db = Some(db.to_string());
        if self.expanded.contains_key(db) {
            self.expanded.remove(db);
            cx.notify();
            return;
        }
        self.expanded
            .insert(db.to_string(), ExpandedState::default());
        self.load_collections(db.to_string(), cx);
        cx.emit(TreeEvent::DatabaseActivated {
            database: db.to_string(),
        });
    }

    fn load_collections(&mut self, db: String, cx: &mut Context<Self>) {
        let Some(conf) = self.connection.clone() else {
            return;
        };
        if let Some(state) = self.expanded.get_mut(&db) {
            state.loading = true;
            state.error = None;
        }
        cx.notify();
        let svc = self.service.clone();
        let db_for_async = db.clone();
        cx.spawn(async move |this, cx| {
            let r = svc.list_collections(&conf, &db_for_async).await;
            let _ = this.update(cx, |this, cx| {
                if let Some(state) = this.expanded.get_mut(&db_for_async) {
                    state.loading = false;
                    match r {
                        Ok(cs) => {
                            info!(db = %db_for_async, count = cs.len(), "mongo collections loaded");
                            state.collections = cs;
                        }
                        Err(e) => {
                            error!(error = %e, db = %db_for_async, "mongo list_collections failed");
                            state.error = Some(e.to_string());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_collection(&mut self, db: String, coll: String, cx: &mut Context<Self>) {
        self.active_db = Some(db.clone());
        self.selected = Some((db.clone(), coll.clone()));
        cx.emit(TreeEvent::CollectionSelected {
            database: db,
            collection: coll,
        });
        cx.notify();
    }

    /// picker 选库：激活 + 确保展开 + 通知（不像 toggle_database 会把已展开的库收起）
    fn select_database(&mut self, db: String, cx: &mut Context<Self>) {
        self.active_db = Some(db.clone());
        if !self.expanded.contains_key(&db) {
            self.expanded.insert(db.clone(), ExpandedState::default());
            self.load_collections(db.clone(), cx);
        }
        cx.emit(TreeEvent::DatabaseActivated { database: db });
        cx.notify();
    }

    fn current_filter(&self, cx: &gpui::App) -> String {
        self.search
            .read(cx)
            .value()
            .to_string()
            .to_ascii_lowercase()
    }
}

impl Render for CollectionTreePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 右键操作（清空/删除）异步完成的 toast 在这里推送
        if let Some(n) = self.pending_notification.take() {
            use gpui_component::WindowExt as _;
            window.push_notification(n, cx);
        }
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted_fg = theme.muted_foreground;
        let border = theme.border;
        let accent = theme.accent;

        let filter = self.current_filter(cx);

        // 顶栏第 1 行：DB picker 下拉（仿 dbclient::table_tree 的 schema picker，点开切换 active_db）
        let show_system = self.show_system;
        let active_label = self
            .active_db
            .clone()
            .unwrap_or_else(|| "未选库".to_string());
        let picker_label = format!("DB {active_label} ▾");
        let entity_for_picker = cx.entity().clone();
        let active_for_menu = self.active_db.clone();
        let picker_dbs: Vec<String> = self
            .databases
            .iter()
            .filter(|d| show_system || !is_system_db(&d.name))
            .map(|d| d.name.clone())
            .collect();
        let header = h_flex()
            .w_full()
            .px(px(10.0))
            .py(px(6.0))
            .border_b_1()
            .border_color(border)
            .items_center()
            .gap(px(8.0))
            .child(
                Button::new("mongo-db-picker")
                    .ghost()
                    .small()
                    .label(picker_label)
                    .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _, _| {
                        let mut m = menu;
                        let active = active_for_menu.clone();
                        for d in &picker_dbs {
                            let d_owned = d.clone();
                            let is_active = active.as_deref() == Some(d.as_str());
                            let label = if is_active {
                                format!("✓ {d}")
                            } else {
                                format!("  {d}")
                            };
                            let entity = entity_for_picker.clone();
                            m = m.item(PopupMenuItem::new(label).on_click(move |_, _, app| {
                                let d = d_owned.clone();
                                entity.update(app, |this, cx| this.select_database(d, cx));
                            }));
                        }
                        m
                    }),
            );

        // 顶栏第 2 行：搜索框 + 三个工具按钮（眼睛 / 刷新 / 命令编辑器切换）—— 与 MySQL 同款布局
        let editor_visible = self.editor_visible;
        let toggle_sys_tip = if show_system {
            "隐藏系统库（admin / config / local）"
        } else {
            "显示系统库（admin / config / local）"
        };
        let toggle_editor_tip = if editor_visible {
            "隐藏命令编辑器 (⌘E)"
        } else {
            "显示命令编辑器 (⌘E)"
        };

        let search_row = h_flex()
            .w_full()
            .items_center()
            .px(px(10.0))
            .py(px(6.0))
            .border_b_1()
            .border_color(border)
            .gap(px(6.0))
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.search).small().cleanable(true).prefix(
                        gpui_component::Icon::new(gpui_component::IconName::Search)
                            .small()
                            .text_color(muted_fg),
                    ),
                ),
            )
            .child(
                Button::new("toggle-system-dbs")
                    .ghost()
                    .xsmall()
                    .icon(if show_system {
                        gpui_component::IconName::Eye
                    } else {
                        gpui_component::IconName::EyeOff
                    })
                    .tooltip(toggle_sys_tip)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_show_system(cx))),
            )
            .child(
                Button::new("refresh-mongo-tree")
                    .ghost()
                    .xsmall()
                    .icon(ramag_ui::icons::refresh_cw())
                    .tooltip("刷新")
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            )
            .child(
                Button::new("toggle-mongo-editor")
                    .ghost()
                    .xsmall()
                    .icon(gpui_component::IconName::SquareTerminal)
                    .selected(editor_visible)
                    .tooltip(toggle_editor_tip)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(TreeEvent::ToggleEditor))),
            );

        // 扁平化树行 → uniform_list 行虚拟化（仿 dbclient::table_tree）
        let tree_rows: Rc<Vec<TreeRow>> = Rc::new(self.build_tree_rows(&filter));
        let _ = (fg, accent); // 颜色由 render_tree_row 内自取
        let body = uniform_list(
            "mongo-tree-rows",
            tree_rows.len(),
            cx.processor({
                let tree_rows = tree_rows.clone();
                move |this, range: Range<usize>, _w, cx| {
                    range
                        .map(|i| this.render_tree_row(&tree_rows[i], cx))
                        .collect::<Vec<_>>()
                }
            }),
        )
        .track_scroll(&self.uniform_scroll)
        .px(px(2.0))
        .py(px(4.0))
        .flex_1();

        // 底部状态栏：「数据库 (可见数/总数)」，与 dbclient::table_tree:403-413 同款
        let total_dbs = self.databases.len();
        let visible_dbs = self
            .databases
            .iter()
            .filter(|db| {
                if !self.show_system && is_system_db(&db.name) {
                    return false;
                }
                if filter.is_empty() {
                    return true;
                }
                let name_lc = db.name.to_ascii_lowercase();
                if name_lc.contains(&filter) {
                    return true;
                }
                // 也算"已展开 db 下任一 collection 名匹配"
                self.expanded
                    .get(&db.name)
                    .map(|s| {
                        s.collections
                            .iter()
                            .any(|c| c.name.to_ascii_lowercase().contains(&filter))
                    })
                    .unwrap_or(false)
            })
            .count();
        let footer_text = if total_dbs == visible_dbs {
            format!("数据库 ({total_dbs})")
        } else {
            format!("数据库 ({visible_dbs}/{total_dbs})")
        };

        v_flex()
            .size_full()
            .overflow_hidden()
            .bg(theme.background)
            .child(header)
            .child(search_row)
            .child(body)
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .px_2()
                    .py(px(4.0))
                    .border_t_1()
                    .border_color(border)
                    .text_xs()
                    .text_color(muted_fg)
                    .child(SharedString::from(footer_text)),
            )
    }
}
