//! ConnectionSession：一个打开的连接会话
//!
//! 每个 Session 对应顶部一个 Tab，内部持有该连接的：
//! - 表树（TableTreePanel）
//! - 查询面板（QueryPanel）
//!
//! 用户切换 Tab 即在不同连接环境间切换。

use std::sync::Arc;
use std::time::Duration;

use gpui::{
    Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window, div,
    prelude::*, px,
};
use gpui_component::{
    ActiveTheme, h_flex,
    resizable::{ResizableState, h_resizable, resizable_panel},
};
use parking_lot::RwLock;
use ramag_app::ConnectionService;
use ramag_domain::entities::{ConnectionConfig, DriverKind};
use tracing::{info, warn};

use crate::sql_completion::{SchemaCache, is_system_schema};
use crate::views::query_panel::QueryPanel;
use crate::views::table_tree::{TableTreePanel, TreeEvent};

/// 补全 cache 的 TTL：超过这个时长后台异步重拉一次
/// 兜底「别人改了表 / 我没看到的 schema」这类 cache 漂移
const CACHE_TTL: Duration = Duration::from_secs(60);

/// 表树初始宽度（用户可拖拽分隔条改）
const TREE_WIDTH_INITIAL: f32 = 280.0;
const TREE_WIDTH_MIN: f32 = 180.0;
const TREE_WIDTH_MAX: f32 = 600.0;

/// 一个连接会话
pub struct ConnectionSession {
    config: ConnectionConfig,
    tree: Entity<TableTreePanel>,
    queries: Entity<QueryPanel>,
    /// 表树 / 查询面板分隔条状态（拖拽改变两侧宽度）
    resize_state: Entity<ResizableState>,
    /// SQL 补全用的 schema 缓存（background 填充；持有 keep-alive，
    /// 实际由 QueryPanel 内部 Tab 通过 Arc 共享读取）
    _schema_cache: Arc<RwLock<SchemaCache>>,
    _subscriptions: Vec<Subscription>,
}

impl ConnectionSession {
    pub fn new(
        config: ConnectionConfig,
        service: Arc<ConnectionService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let schema_cache = SchemaCache::new_shared();
        // 默认 schema 立即记录
        schema_cache.write().default_schema = config.database.clone();

        let tree =
            cx.new(|cx| TableTreePanel::new(service.clone(), schema_cache.clone(), window, cx));
        let queries =
            cx.new(|cx| QueryPanel::new(service.clone(), schema_cache.clone(), window, cx));

        // 立即设置连接 → 加载 schemas + 同步 queries
        let conn_for_tree = config.clone();
        tree.update(cx, |t, cx| t.set_connection(Some(conn_for_tree), cx));
        let conn_for_q = config.clone();
        queries.update(cx, |q, cx| q.set_connection(Some(conn_for_q), cx));

        // 后台拉表名填补全 cache（默认 schema 优先；无默认时拉所有非系统库）
        Self::warm_schema_cache(service.clone(), config.clone(), schema_cache.clone(), cx);
        // 启动 TTL 周期任务：每 60s 重新拉一次，兜底外部修改造成的漂移
        Self::start_cache_ttl(service.clone(), config.clone(), schema_cache.clone(), cx);

        let mut subs = Vec::new();

        // 订阅表树事件：填 SELECT 到当前 Tab 并自动执行；同时把 schema
        // 同步到所有 Tab（写裸表名 SQL 时不会再报 No database selected）
        let queries_clone = queries.clone();
        let tree_for_sync = tree.clone();
        // 当前 session 的 driver（mysql/pg），订阅闭包内决定方言写法时用
        let driver_kind = config.driver;
        subs.push(cx.subscribe_in(
            &tree,
            window,
            move |_this: &mut Self, _, e: &TreeEvent, window, cx| match e {
                TreeEvent::TableSelected { schema, table } => {
                    info!(schema = %schema, table = %table, "table selected, prefill + run");
                    queries_clone.update(cx, |q, cx| {
                        q.set_active_schema(Some(schema.clone()), cx);
                    });
                    // 按 driver 方言加引号（mysql 反引号 / pg 双引号）
                    let qschema = driver_kind.quote_identifier(schema);
                    let qtable = driver_kind.quote_identifier(table);
                    let sql = format!(
                        "SELECT * FROM {qschema}.{qtable} LIMIT {};",
                        super::query_tab::AUTO_LIMIT,
                    );
                    let target = Some((schema.clone(), table.clone()));
                    queries_clone.update(cx, |q, cx| {
                        q.prefill_active_sql_and_run_with_target(sql, target, window, cx)
                    });
                }
                TreeEvent::SchemaActivated { schema } => {
                    info!(schema = %schema, "schema activated");
                    queries_clone.update(cx, |q, cx| {
                        q.set_active_schema(Some(schema.clone()), cx);
                    });
                }
                TreeEvent::ShowCreateTable {
                    schema,
                    table,
                    is_view,
                } => {
                    info!(schema = %schema, table = %table, is_view, "show create");
                    // 按 driver 选 DDL 查询语句（mysql SHOW CREATE / pg 拼装版）
                    let sql = super::ddl::build_ddl_query(driver_kind, schema, table, *is_view);
                    queries_clone.update(cx, |q, cx| {
                        q.open_in_new_tab_and_run(sql, window, cx);
                    });
                }
                TreeEvent::ToggleSqlEditor => {
                    // 只切 QueryPanel 内的 SQL 编辑器；下方工具条/结果表格保留
                    let visible = queries_clone.update(cx, |q, cx| q.toggle_editor(cx));
                    info!(visible, "toggle sql editor");
                    // 同步给 tree，让按钮图标朝向匹配
                    tree_for_sync.update(cx, |t, cx| t.set_editor_visible(visible, cx));
                }
            },
        ));

        let resize_state = cx.new(|_| ResizableState::default());

        Self {
            config,
            tree,
            queries,
            resize_state,
            _schema_cache: schema_cache,
            _subscriptions: subs,
        }
    }

    /// 后台预拉一次 schema → tables 填补全 cache
    fn warm_schema_cache(
        service: Arc<ConnectionService>,
        config: ConnectionConfig,
        cache: Arc<RwLock<SchemaCache>>,
        cx: &mut Context<Self>,
    ) {
        cx.background_spawn(async move {
            warm_once(&service, &config, &cache).await;
        })
        .detach();
    }

    /// TTL 周期刷新：每 CACHE_TTL 后台拉一次最新表名
    /// 通过 this.update 检测 entity 是否已 drop，drop 后自动退出循环
    fn start_cache_ttl(
        service: Arc<ConnectionService>,
        config: ConnectionConfig,
        cache: Arc<RwLock<SchemaCache>>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CACHE_TTL).await;
                // session drop 后退出 ticker
                if this.update(cx, |_, _| ()).is_err() {
                    break;
                }
                warm_once(&service, &config, &cache).await;
            }
        })
        .detach();
    }

    pub fn config(&self) -> &ConnectionConfig {
        &self.config
    }

    /// Tab 标题（连接名）
    pub fn title(&self) -> &str {
        &self.config.name
    }

    /// 数据库类型副标题（用于 Tab Bar 二级展示）
    pub fn kind_label(&self) -> &'static str {
        match self.config.driver {
            DriverKind::Mysql => "MySQL",
            DriverKind::Postgres => "PostgreSQL",
            DriverKind::Redis => "Redis",
        }
    }
}

impl Render for ConnectionSession {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        // h_resizable 让用户拖拽中间分隔条调整左右宽度
        // 表树初始 280px，限制 [180, 600]；查询面板占剩余
        h_flex()
            .size_full()
            .bg(theme.background)
            // ⌘E：切 SQL 编辑器（动作走 dispatch 冒泡到此）
            // 与表树按钮 emit 的 ToggleSqlEditor 殊途同归，都调 queries.toggle_editor + 同步 tree
            .on_action(
                cx.listener(|this, _: &crate::actions::ToggleSqlEditor, _, cx| {
                    let visible = this.queries.update(cx, |q, cx| q.toggle_editor(cx));
                    this.tree
                        .update(cx, |t, cx| t.set_editor_visible(visible, cx));
                }),
            )
            .child(
                h_resizable("session-resize")
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

/// 实际刷新逻辑：异步拉一次目标 schema 的所有表名 → 写入 cache
/// 初次预热与 TTL 周期任务都用这一份
async fn warm_once(
    service: &ConnectionService,
    config: &ConnectionConfig,
    cache: &Arc<RwLock<SchemaCache>>,
) {
    let target_schemas: Vec<String> = if let Some(db) = &config.database {
        vec![db.clone()]
    } else {
        match service.list_schemas(config).await {
            Ok(ss) => ss
                .into_iter()
                .map(|s| s.name)
                .filter(|n| !is_system_schema(n))
                .collect(),
            Err(e) => {
                warn!(error = %e, "warm cache: list_schemas failed");
                return;
            }
        }
    };
    for schema in target_schemas {
        match service.list_tables(config, &schema).await {
            Ok(tables) => {
                let names: Vec<String> = tables.into_iter().map(|t| t.name).collect();
                cache.write().tables.insert(schema, names);
            }
            Err(e) => {
                warn!(error = %e, "warm cache: list_tables failed");
            }
        }
    }
    info!(
        schemas = cache.read().tables.len(),
        "schema cache refreshed"
    );
}
