//! 单个查询标签：编辑器 + 工具条 + 结果面板

mod actions;
mod examples;
mod paging;
mod render;
mod sql_utils;

// QueryPanel 的 Tab 栏「示例」下拉用
pub(crate) use examples::sql_examples;

use std::sync::Arc;
use std::time::Instant;

use gpui::{AppContext as _, Context, Entity, Task, Window};
use gpui_component::input::{InputEvent, InputState};
use gpui_component::notification::Notification;
use parking_lot::RwLock;

use ramag_app::ConnectionService;
use ramag_domain::entities::ConnectionConfig;

use crate::sql_completion::SchemaCache;
use crate::views::result_panel::ResultPanel;

/// 单个查询标签
pub struct QueryTab {
    pub(super) service: Arc<ConnectionService>,
    /// 当前激活的连接（None 时禁用执行）
    pub(super) connection: Option<ConnectionConfig>,
    /// 当前激活的默认库；表树点击表/schema 时由父 session 同步进来
    pub(super) active_schema: Option<String>,
    /// SQL 编辑器
    pub(super) editor: Entity<InputState>,
    /// 结果面板
    pub(super) result: Entity<ResultPanel>,
    /// 是否在执行中
    pub(super) running: bool,
    /// 当前正在跑的任务句柄（drop 后取消异步任务）
    pub(super) current_task: Option<Task<()>>,
    /// 取消句柄：driver 在 acquire 后写入 mysql 后端 thread id（0 = 未拿到）
    pub(super) cancel_handle: Option<ramag_domain::traits::CancelHandle>,
    /// 查询开始时间，仅 running 时为 Some
    pub(super) query_start: Option<Instant>,
    /// 与编辑器 / 表树共享的补全 schema 缓存（用于 DDL 后自动刷新）
    pub(super) schema_cache: Arc<RwLock<SchemaCache>>,
    /// Tab 标题（默认值，如 "Query 1"）
    pub(super) title: String,
    /// 上次执行的 SQL 摘要：成功执行后从 SQL 派生
    pub(super) short_title: Option<String>,
    /// 异步任务（如保存文件）完成后挂这里，下次 render 在 window 上推送
    pub(super) pending_notification: Option<Notification>,
    /// 上游显式指定的目标表 (schema, table)：表树点击触发的 SELECT 才有
    pub(super) pinned_target: Option<(String, String)>,
    /// 是否显示 SQL 编辑器
    pub(super) show_editor: bool,
    /// 自动 LIMIT 注入开关
    pub(super) auto_limit_enabled: bool,
    /// 分页状态：本次 run 命中"未手写 LIMIT 的单条 SELECT"时为 Some
    pub(super) pager: Option<paging::Pager>,
    /// 编辑器变化订阅 keep-alive
    pub(super) _editor_sub: gpui::Subscription,
}

impl QueryTab {
    pub fn new(
        service: Arc<ConnectionService>,
        title: impl Into<String>,
        connection: Option<ConnectionConfig>,
        schema_cache: Arc<RwLock<SchemaCache>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let cache_for_provider = schema_cache.clone();
        let editor = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .code_editor("sql")
                .multi_line(true)
                .line_number(true)
                .placeholder("-- 输入 SQL，按 ⌘↵ 运行\nSELECT 1;")
                .rows(8);
            // SQL 补全：关键字 + 表名 + 列名（cache 共享）
            state.lsp.completion_provider = Some(
                crate::sql_completion::SqlCompletionProvider::new_rc(cache_for_provider),
            );
            state
        });
        let cache_for_result = schema_cache.clone();
        let result = cx.new(|cx| {
            let mut p = ResultPanel::new(window, cx);
            // 把执行器注入：单元格编辑弹框「确认修改」需要异步发 UPDATE
            p.set_executor(Some(service.clone()), connection.clone());
            // schema cache：判断 current_table 是否视图，从而禁用写按钮
            p.set_schema_cache(Some(cache_for_result));
            p
        });

        // 订阅编辑器内容变化：发现新提到的表 → 后台预拉它的列结构
        let editor_sub = cx.subscribe(&editor, |this: &mut Self, _, e: &InputEvent, cx| {
            if matches!(e, InputEvent::Change) {
                this.prefetch_columns_for_used_tables(cx);
            }
        });

        let initial_schema = connection
            .as_ref()
            .and_then(|c| c.database.clone())
            .filter(|s| !s.is_empty());
        Self {
            service,
            connection,
            active_schema: initial_schema,
            editor,
            result,
            running: false,
            current_task: None,
            cancel_handle: None,
            query_start: None,
            schema_cache,
            title: title.into(),
            short_title: None,
            pending_notification: None,
            pinned_target: None,
            show_editor: true,
            // 默认开启 LIMIT 自动注入；用户可在工具条切换
            auto_limit_enabled: true,
            pager: None,
            _editor_sub: editor_sub,
        }
    }

    /// 由 QueryPanel 全局同步：是否展示顶部 SQL 编辑器
    pub fn set_show_editor(&mut self, v: bool, cx: &mut Context<Self>) {
        if self.show_editor != v {
            self.show_editor = v;
            cx.notify();
        }
    }

    /// 切换表时调：清空结果集的列/行过滤框，避免旧过滤条件遮挡新表数据
    pub fn clear_result_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.result.update(cx, |r, cx| r.clear_filters(window, cx));
    }

    /// 上游设定/清除当前 Tab 的目标表（仅表树点击会注入；手动 run 不变）
    pub fn set_pinned_target(&mut self, target: Option<(String, String)>) {
        self.pinned_target = target;
    }

    /// 用于 TabBar 展示的标题：上次成功执行的 SQL 摘要 > 默认 Tab 名
    pub fn display_title(&self) -> &str {
        self.short_title.as_deref().unwrap_or(&self.title)
    }

    pub fn set_connection(&mut self, conn: Option<ConnectionConfig>, cx: &mut Context<Self>) {
        // 切换连接时把默认库重置成新连接的 database 字段
        self.active_schema = conn
            .as_ref()
            .and_then(|c| c.database.clone())
            .filter(|s| !s.is_empty());
        self.connection = conn.clone();
        // 旧连接的分页状态不能带到新连接（base_sql 已不可信）
        self.pager = None;
        // 同步给 ResultPanel：单元格编辑弹框需要最新的连接来发 UPDATE
        let svc = self.service.clone();
        self.result.update(cx, |r, _| {
            r.set_executor(Some(svc), conn);
        });
        cx.notify();
    }

    /// 父级（ConnectionSession）同步当前活动库；点表树会调用
    pub fn set_active_schema(&mut self, schema: Option<String>, cx: &mut Context<Self>) {
        let normalized = schema.filter(|s| !s.is_empty());
        if self.active_schema != normalized {
            self.active_schema = normalized;
            cx.notify();
        }
    }

    /// 把 SQL 写入编辑器（替换原有内容）
    pub fn set_sql(
        &mut self,
        sql: impl Into<gpui::SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editor
            .update(cx, |state, cx| state.set_value(sql, window, cx));
        // 用户改了 SQL 就清掉之前的 pinned_target：行内编辑不应再用旧目标表
        self.pinned_target = None;
        // 编辑器被整体替换后旧分页状态作废（避免"下一页"重跑已被换掉的 SQL）
        self.pager = None;
        // set_value 不发 InputEvent::Change（emit_events=false），手动触发预拉
        self.prefetch_columns_for_used_tables(cx);
        cx.notify();
    }

    /// 对外暴露：让其他视图（如点表树后）触发执行
    pub fn run(&mut self, cx: &mut Context<Self>) {
        self.handle_run(cx);
    }

    /// 把示例 SQL 写入编辑器：空编辑器整体替换，非空在光标处插入并按前后文补换行，
    /// 避免文首插入产生空行、或与既有语句粘在同一行
    pub(super) fn insert_example(
        &mut self,
        sql: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (value, cursor) = {
            let state = self.editor.read(cx);
            (state.value().to_string(), state.cursor())
        };
        self.editor.update(cx, |state, cx| {
            if value.trim().is_empty() {
                state.set_value(sql.to_string(), window, cx);
            } else {
                // cursor 是 byte offset，防御性对齐到 char 边界
                let mut at = cursor.min(value.len());
                while at > 0 && !value.is_char_boundary(at) {
                    at -= 1;
                }
                let text = examples::wrap_for_insert(&value[..at], &value[at..], sql);
                state.insert(text, window, cx);
            }
            state.focus(window, cx);
        });
        // set_value / insert 不发 Change 事件，手动触发列结构预拉（与 set_sql 一致）
        self.prefetch_columns_for_used_tables(cx);
        cx.notify();
    }

    /// 聚焦编辑器（关闭 / 切换 Tab 后由 QueryPanel 调用，避免用户再点一下）
    pub fn focus_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |state, cx| {
            state.focus(window, cx);
        });
    }
}
