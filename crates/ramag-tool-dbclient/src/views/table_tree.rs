//! 表树面板：显示某连接下的所有 schema 和 tables
//!
//! Stage 2 版本：两级树（schema → tables），点 table 高亮但不做后续动作。

use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, AppContext as _, ClickEvent, Context, EventEmitter, IntoElement, ParentElement,
    Render, SharedString, Styled, UniformListScrollHandle, Window, div, prelude::*, px,
    uniform_list,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem},
    v_flex,
};
use parking_lot::RwLock;
use ramag_app::ConnectionService;
use ramag_domain::entities::{Column, ConnectionConfig, ForeignKey, Index, Schema};

use crate::sql_completion::{SchemaCache, is_system_schema};

use super::tree_helpers::{format_thousands, render_column_row, render_columns_placeholder};
use tracing::error;

pub struct TableTreePanel {
    service: Arc<ConnectionService>,
    connection: Option<ConnectionConfig>,
    loading_schemas: bool,
    schemas: Vec<Schema>,
    error: Option<String>,
    expanded: HashMap<String, SchemaTables>,
    /// 已展开的表 → 列状态（key 为 "schema.table"）
    table_columns: HashMap<String, TableColumns>,
    selected: Option<(String, String)>,
    /// 是否显示系统库（默认隐藏）
    show_system: bool,
    /// 搜索输入（按名称过滤 schema 和 table）
    search: gpui::Entity<InputState>,
    /// SQL 补全的 schema 缓存：展开 schema / 表时把数据顺手写进去，
    /// 用户看到什么补全里就有什么
    schema_cache: Arc<RwLock<SchemaCache>>,
    /// 父级 (QueryPanel via session) 注入：当前 SQL 编辑器是否可见
    /// 仅用于让 toggle 按钮显示正确朝向（PanelRightOpen / PanelRightClose）
    editor_visible: bool,
    /// 当前激活的 schema（与 Redis 的 DB picker 对齐）
    /// - schema 行点击时同步设置
    /// - 顶部 picker 下拉切换时设置
    /// - 显示在树顶部按钮 label 上
    active_schema: Option<String>,
    /// 树体虚拟列表滚动句柄：扁平化为 Vec<TreeRow> 后用 uniform_list
    /// 行级虚拟化，万级 schema/table 也流畅（与 Redis Key 树同款方案）
    uniform_scroll: UniformListScrollHandle,
    _subscriptions: Vec<gpui::Subscription>,
}

#[derive(Default)]
struct SchemaTables {
    loading: bool,
    tables: Vec<ramag_domain::entities::Table>,
    error: Option<String>,
}

#[derive(Default)]
struct TableColumns {
    loading: bool,
    columns: Vec<Column>,
    indexes: Vec<Index>,
    foreign_keys: Vec<ForeignKey>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum TreeEvent {
    /// 用户点了表（高亮 + 父级用 schema 设置默认库 + 自动 SELECT *）
    TableSelected { schema: String, table: String },
    /// 用户点了 schema 行（仅切换默认库，不执行任何 SQL）
    SchemaActivated { schema: String },
    /// 用户点了表/视图行的 DDL 按钮
    /// is_view=true → 父级跑 SHOW CREATE VIEW；false → SHOW CREATE TABLE
    ShowCreateTable {
        schema: String,
        table: String,
        is_view: bool,
    },
    /// 表树 header 切换 SQL 编辑器（仅编辑器；工具条/结果表格保留）
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
        let mut subs = Vec::new();
        subs.push(cx.subscribe(&search, |_this, _, _e: &InputEvent, cx| cx.notify()));

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
            // 数据浏览/导出是主场景，写 SQL 时按 ⌘E 或点按钮唤出
            editor_visible: false,
            active_schema: None,
            uniform_scroll: UniformListScrollHandle::new(),
            _subscriptions: subs,
        }
    }

    /// 父级（QueryPanel）通知 SQL 编辑器当前显隐状态
    /// 仅用于 toggle 按钮的图标朝向；实际显隐由 QueryPanel/QueryTab 管
    pub fn set_editor_visible(&mut self, v: bool, cx: &mut Context<Self>) {
        if self.editor_visible != v {
            self.editor_visible = v;
            cx.notify();
        }
    }

    fn current_filter(&self, cx: &gpui::App) -> String {
        self.search
            .read(cx)
            .value()
            .to_string()
            .to_ascii_lowercase()
    }

    fn toggle_show_system(&mut self, cx: &mut Context<Self>) {
        self.show_system = !self.show_system;
        // 同步到共享 cache：DB 下拉根据此值决定是否展示系统库
        self.schema_cache.write().show_system = self.show_system;
        cx.notify();
    }

    /// 强制刷新：清空已展开/已缓存的表结构，重新拉 schema 列表
    fn refresh(&mut self, cx: &mut Context<Self>) {
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

    fn load_schemas(&mut self, cx: &mut Context<Self>) {
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

    fn toggle_schema(&mut self, schema_name: String, cx: &mut Context<Self>) {
        // 不论展开还是收起，都把"当前 schema"广播给父级（设默认库）+ 自身 active_schema
        // active_schema 决定顶部 picker 显示文本
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
                        // 顺手把表名同步到补全 cache（一次 IO 两份用）
                        let names: Vec<String> = tables.iter().map(|t| t.name.clone()).collect();
                        this.schema_cache
                            .write()
                            .tables
                            .insert(schema_for_async.clone(), names);
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

    fn handle_table_click(&mut self, schema: String, table: String, cx: &mut Context<Self>) {
        self.selected = Some((schema.clone(), table.clone()));
        cx.emit(TreeEvent::TableSelected { schema, table });
        cx.notify();
    }

    /// 表/视图行 DDL 按钮点击：让父级（ConnectionSession）跑 SHOW CREATE TABLE 或 SHOW CREATE VIEW
    fn handle_show_ddl(
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
    fn toggle_table_columns(&mut self, schema: String, table: String, cx: &mut Context<Self>) {
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
                        // 列名同步到补全 cache（Phase 3 列名补全的数据来源）
                        let col_names: Vec<String> =
                            cols.iter().map(|c| c.name.clone()).collect();
                        this.schema_cache.write().columns.insert(
                            (schema_async.clone(), table_async.clone()),
                            col_names,
                        );
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

/// 扁平化的树行（uniform_list 行级虚拟化的数据单元）
///
/// 所有变体的渲染高度统一为 28px（由 render_tree_row 与 tree_helpers 的两个
/// 占位/列函数共同保证）。这是 uniform_list 行级度量的硬约束 ——
/// 高度不一致会让虚拟列表计算错位
#[derive(Clone)]
enum TreeRow {
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
        row_estimate: Option<u64>,
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
    ///
    /// `+ use<>`：避免捕获 &self 与 &mut cx 的 lifetime 让返回值挂钩外层借用，
    /// 使 closure 内重复调用时不互相干扰
    fn render_tree_row(&self, row: &TreeRow, cx: &mut Context<Self>) -> AnyElement {
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
                row_estimate,
            } => {
                let schema = schema.clone();
                let name = name.clone();
                let is_view = *is_view;
                let is_cols_expanded = *is_cols_expanded;
                let is_selected = *is_selected;
                let row_estimate = *row_estimate;

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
                    // chevron 单击只展开列结构，不触发 TableSelected（保留原 stop_propagation 行为）
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
                if let Some(n) = row_estimate {
                    row = row.child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .flex_none()
                            .child(format!("(~{})", format_thousands(n))),
                    );
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

impl Render for TableTreePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 仅 render() 当前 scope 直接用到的颜色（早期返回 + header bar 文字）；
        // 行渲染需要的颜色由 render_tree_row 自己 cx.theme() 取
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
                // 没有 filter 时全部通过；
                // 有 filter 时：schema 名匹配 OR 该 schema 下任意已展开的 table 匹配
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
        // 排序：业务库优先，系统库统一沉到底部；同组按名字字典序
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
        let schema_count = schemas.len();

        // 扁平化为 Vec<TreeRow> 喂给 uniform_list 行级虚拟化（28px 等高）
        let mut tree_rows: Vec<TreeRow> = Vec::with_capacity(schemas.len() * 4);
        let total_schemas = self.schemas.len();
        let visible_schemas = schemas.len();
        let _ = schema_count; // 未使用变量
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
        // 切换 SQL 编辑器（只控编辑器+TabBar；工具条/结果表格保留）
        // 快捷键：⌘E。固定用 SquareTerminal 图标（语义=代码/SQL 编辑器），
        // 显隐通过 selected 高亮区分，不再切换图标本身
        let qp_visible = self.editor_visible;
        let qp_tip = if qp_visible {
            "隐藏 SQL 编辑器 (⌘E)"
        } else {
            "显示 SQL 编辑器 (⌘E)"
        };
        // 顶部第 1 行：schema picker（与 Redis 的 DB picker 对齐布局）
        // 显示当前 active schema；下拉列出 schemas 切换；空态显示"未选库"
        // 系统库 toggle 控制下拉里是否显示系统库
        let active_label = self
            .active_schema
            .clone()
            .unwrap_or_else(|| "未选库".to_string());
        let picker_label = format!("DB {active_label} ▾");
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

        // 顶部第 2 行：搜索框 + 三个工具按钮（与 Redis Key 树同款样式）
        // 搜索框：small 尺寸 + cleanable X + 放大镜 prefix
        let header_bar = h_flex()
            .w_full()
            .items_center()
            .px(px(10.0))
            .py(px(8.0))
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
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        cx.emit(TreeEvent::ToggleSqlEditor);
                        let _ = this;
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
                    // metadata.list_tables 已按 TABLE_TYPE,TABLE_NAME 排序，这里再用 partition 算分组数量
                    // 仅当过滤后两类都非空时才显示分组标题（只有一类时不啰嗦）
                    let total_tables = tables.iter().filter(|t| !t.is_view).count();
                    let total_views = tables.iter().filter(|t| t.is_view).count();
                    let show_group_header = total_tables > 0 && total_views > 0;
                    let mut last_was_view: Option<bool> = None;
                    for t in tables.iter() {
                        // 搜索过滤：搜索时只显示匹配的 table
                        if has_filter
                            && !name.to_ascii_lowercase().contains(&filter)
                            && !t.name.to_ascii_lowercase().contains(&filter)
                        {
                            continue;
                        }
                        // 切换分组时插入 header（仅基础表+视图共存时显示）
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
                            row_estimate: t.row_estimate,
                        });

                        // 展开的列结构子节点 + 索引 + 外键
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

        // uniform_list 行级虚拟化：仅渲染屏幕可见行，万级 schema/table 也流畅
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
            // 顶部第 1 行：schema picker（DB: <name> ▾）
            .child(db_row)
            // 顶部第 2 行：搜索框 + 三按钮（与 Redis Key 树布局一致）
            .child(header_bar)
            // 树体（虚拟列表）
            .child(body)
            // 底部 status bar：数据库计数
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
