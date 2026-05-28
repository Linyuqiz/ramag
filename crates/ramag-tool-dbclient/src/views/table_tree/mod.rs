//! 表树面板：连接下的 schema → tables

mod render;
mod row;

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{AppContext as _, Context, EventEmitter, UniformListScrollHandle, Window};
use gpui_component::input::{InputEvent, InputState};
use parking_lot::RwLock;
use ramag_app::ConnectionService;
use ramag_domain::entities::{Column, ConnectionConfig, DriverKind, ForeignKey, Index, Schema};
use tracing::error;

use crate::sql_completion::{SchemaCache, is_system_schema};

pub struct TableTreePanel {
    pub(super) service: Arc<ConnectionService>,
    pub(super) connection: Option<ConnectionConfig>,
    pub(super) loading_schemas: bool,
    pub(super) schemas: Vec<Schema>,
    pub(super) error: Option<String>,
    pub(super) expanded: HashMap<String, SchemaTables>,
    /// 已展开的表 → 列状态（key 为 "schema.table"）
    pub(super) table_columns: HashMap<String, TableColumns>,
    pub(super) selected: Option<(String, String)>,
    /// 是否显示系统库（默认隐藏）
    pub(super) show_system: bool,
    /// 搜索输入（按名称过滤 schema 和 table）
    pub(super) search: gpui::Entity<InputState>,
    /// SQL 补全的 schema 缓存
    pub(super) schema_cache: Arc<RwLock<SchemaCache>>,
    /// 父级（QueryPanel via session）注入：当前 SQL 编辑器是否可见
    pub(super) editor_visible: bool,
    /// 当前激活的 schema
    pub(super) active_schema: Option<String>,
    /// 树体虚拟列表滚动句柄
    pub(super) uniform_scroll: UniformListScrollHandle,
    pub(super) _subscriptions: Vec<gpui::Subscription>,
}

#[derive(Default)]
pub(super) struct SchemaTables {
    pub(super) loading: bool,
    pub(super) tables: Vec<ramag_domain::entities::Table>,
    pub(super) error: Option<String>,
}

#[derive(Default)]
pub(super) struct TableColumns {
    pub(super) loading: bool,
    pub(super) columns: Vec<Column>,
    pub(super) indexes: Vec<Index>,
    pub(super) foreign_keys: Vec<ForeignKey>,
    pub(super) error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum TreeEvent {
    /// 用户点了表（高亮 + 父级用 schema 设置默认库 + 自动 SELECT *）
    TableSelected { schema: String, table: String },
    /// 用户点了 schema 行（仅切换默认库，不执行任何 SQL）
    SchemaActivated { schema: String },
    /// 用户点了表/视图行的 DDL 按钮
    ShowCreateTable {
        schema: String,
        table: String,
        is_view: bool,
    },
    /// 表树 header 切换 SQL 编辑器
    ToggleSqlEditor,
}

impl EventEmitter<TreeEvent> for TableTreePanel {}

impl TableTreePanel {
    pub fn new(
        service: Arc<ConnectionService>,
        schema_cache: Arc<RwLock<SchemaCache>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("搜索 schema / table")
                .clean_on_escape()
        });
        // 搜索框文本变化时重渲染
        let subs = vec![cx.subscribe(&search, |_this, _, _e: &InputEvent, cx| cx.notify())];

        Self {
            service,
            connection: None,
            loading_schemas: false,
            schemas: Vec::new(),
            error: None,
            expanded: HashMap::new(),
            table_columns: HashMap::new(),
            selected: None,
            show_system: false,
            search,
            schema_cache,
            // 默认 false：与 QueryPanel.show_editor 默认值保持一致
            editor_visible: false,
            active_schema: None,
            uniform_scroll: UniformListScrollHandle::new(),
            _subscriptions: subs,
        }
    }

    /// 父级（QueryPanel）通知 SQL 编辑器当前显隐状态
    pub fn set_editor_visible(&mut self, v: bool, cx: &mut Context<Self>) {
        if self.editor_visible != v {
            self.editor_visible = v;
            cx.notify();
        }
    }

    pub(super) fn current_filter(&self, cx: &gpui::App) -> String {
        self.search
            .read(cx)
            .value()
            .to_string()
            .to_ascii_lowercase()
    }

    pub(super) fn toggle_show_system(&mut self, cx: &mut Context<Self>) {
        self.show_system = !self.show_system;
        // 同步到共享 cache：DB 下拉根据此值决定是否展示系统库
        self.schema_cache.write().show_system = self.show_system;
        cx.notify();
    }

    /// 强制刷新：清空已展开/已缓存的表结构，重新拉 schema 列表
    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.connection.is_none() {
            return;
        }
        self.expanded.clear();
        self.table_columns.clear();
        self.selected = None;
        self.error = None;
        self.load_schemas(cx);
    }

    pub fn set_connection(&mut self, conn: Option<ConnectionConfig>, cx: &mut Context<Self>) {
        self.connection = conn;
        self.schemas.clear();
        self.expanded.clear();
        self.table_columns.clear();
        self.selected = None;
        self.error = None;
        if self.connection.is_some() {
            self.load_schemas(cx);
        } else {
            cx.notify();
        }
    }

    pub(super) fn load_schemas(&mut self, cx: &mut Context<Self>) {
        let Some(conn) = self.connection.clone() else {
            return;
        };
        self.loading_schemas = true;
        self.error = None;
        cx.notify();

        let svc = self.service.clone();
        cx.spawn(async move |this, cx| {
            let result = svc.list_schemas(&conn).await;
            let _ = this.update(cx, |this, cx| {
                this.loading_schemas = false;
                match result {
                    Ok(schemas) => {
                        // 写入共享 cache：DB 下拉的选项来自此处
                        let names: Vec<String> = schemas.iter().map(|s| s.name.clone()).collect();
                        this.schema_cache.write().all_schemas = names;
                        this.schemas = schemas;
                        // 首次加载完成后自动激活默认 schema
                        if this.active_schema.is_none()
                            && let Some(default_name) = pick_default_schema(&conn, &this.schemas)
                        {
                            this.toggle_schema(default_name, cx);
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "list schemas failed");
                        this.error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn toggle_schema(&mut self, schema_name: String, cx: &mut Context<Self>) {
        // 不论展开还是收起，都把"当前 schema"广播给父级（设默认库）+ 自身 active_schema
        self.active_schema = Some(schema_name.clone());
        cx.emit(TreeEvent::SchemaActivated {
            schema: schema_name.clone(),
        });

        if self.expanded.remove(&schema_name).is_some() {
            cx.notify();
            return;
        }
        self.expanded.insert(
            schema_name.clone(),
            SchemaTables {
                loading: true,
                ..Default::default()
            },
        );
        cx.notify();

        let Some(conn) = self.connection.clone() else {
            return;
        };
        let svc = self.service.clone();
        let schema_for_async = schema_name.clone();
        cx.spawn(async move |this, cx| {
            let result = svc.list_tables(&conn, &schema_for_async).await;
            let _ = this.update(cx, |this, cx| {
                let entry = this.expanded.entry(schema_for_async.clone()).or_default();
                entry.loading = false;
                match result {
                    Ok(tables) => {
                        let names: Vec<String> = tables.iter().map(|t| t.name.clone()).collect();
                        let view_set: std::collections::HashSet<String> = tables
                            .iter()
                            .filter(|t| t.is_view)
                            .map(|t| t.name.clone())
                            .collect();
                        {
                            let mut cache = this.schema_cache.write();
                            cache.tables.insert(schema_for_async.clone(), names);
                            cache.views.insert(schema_for_async.clone(), view_set);
                        }
                        entry.tables = tables;
                    }
                    Err(e) => {
                        error!(error = %e, schema = %schema_for_async, "list tables failed");
                        entry.error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn handle_table_click(
        &mut self,
        schema: String,
        table: String,
        cx: &mut Context<Self>,
    ) {
        self.selected = Some((schema.clone(), table.clone()));
        if self.active_schema.as_deref() != Some(schema.as_str()) {
            self.active_schema = Some(schema.clone());
            cx.emit(TreeEvent::SchemaActivated {
                schema: schema.clone(),
            });
        }
        cx.emit(TreeEvent::TableSelected { schema, table });
        cx.notify();
    }

    /// 表/视图行 DDL 按钮点击：让父级（ConnectionSession）跑 SHOW CREATE TABLE 或 SHOW CREATE VIEW
    pub(super) fn handle_show_ddl(
        &mut self,
        schema: String,
        table: String,
        is_view: bool,
        cx: &mut Context<Self>,
    ) {
        cx.emit(TreeEvent::ShowCreateTable {
            schema,
            table,
            is_view,
        });
    }

    /// 切换表的列展开状态：第一次展开时异步拉列结构，关闭只是移除状态
    pub(super) fn toggle_table_columns(
        &mut self,
        schema: String,
        table: String,
        cx: &mut Context<Self>,
    ) {
        let key = format!("{schema}.{table}");
        if self.table_columns.remove(&key).is_some() {
            cx.notify();
            return;
        }

        self.table_columns.insert(
            key.clone(),
            TableColumns {
                loading: true,
                ..Default::default()
            },
        );
        cx.notify();

        let Some(conn) = self.connection.clone() else {
            return;
        };
        let svc = self.service.clone();
        let schema_async = schema.clone();
        let table_async = table.clone();
        cx.spawn(async move |this, cx| {
            // 三类元数据并发拉，索引/外键失败只 warn 不阻塞列结构
            let cols_fut = svc.list_columns(&conn, &schema_async, &table_async);
            let idx_fut = svc.list_indexes(&conn, &schema_async, &table_async);
            let fk_fut = svc.list_foreign_keys(&conn, &schema_async, &table_async);
            let (cols_res, idx_res, fk_res) = futures::join!(cols_fut, idx_fut, fk_fut);
            let _ = this.update(cx, |this, cx| {
                let entry = this
                    .table_columns
                    .entry(key.clone())
                    .or_insert_with(TableColumns::default);
                entry.loading = false;
                match cols_res {
                    Ok(cols) => {
                        let col_names: Vec<String> =
                            cols.iter().map(|c| c.name.clone()).collect();
                        this.schema_cache
                            .write()
                            .columns
                            .insert((schema_async.clone(), table_async.clone()), col_names);
                        entry.columns = cols;
                    }
                    Err(e) => {
                        error!(error = %e, schema = %schema_async, table = %table_async, "list columns failed");
                        entry.error = Some(e.to_string());
                    }
                }
                match idx_res {
                    Ok(ix) => entry.indexes = ix,
                    Err(e) => {
                        tracing::warn!(error = %e, "list indexes failed (non-fatal)");
                    }
                }
                match fk_res {
                    Ok(fk) => entry.foreign_keys = fk,
                    Err(e) => {
                        tracing::warn!(error = %e, "list foreign keys failed (non-fatal)");
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// PG：`public` > 首个非系统；MySQL：config.database > 首个非系统；Redis：None
fn pick_default_schema(conn: &ConnectionConfig, schemas: &[Schema]) -> Option<String> {
    let first_user_schema = || {
        schemas
            .iter()
            .find(|s| !is_system_schema(&s.name))
            .map(|s| s.name.clone())
    };
    match conn.driver {
        DriverKind::Postgres => {
            if schemas.iter().any(|s| s.name == "public") {
                Some("public".to_string())
            } else {
                first_user_schema()
            }
        }
        DriverKind::Mysql => {
            if let Some(db) = conn.database.as_deref().filter(|s| !s.is_empty())
                && schemas.iter().any(|s| s.name == db)
            {
                Some(db.to_string())
            } else {
                first_user_schema()
            }
        }
        DriverKind::Redis | DriverKind::Mongodb => None,
    }
}
