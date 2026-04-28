//! 单个查询标签的视图：编辑器 + 工具条 + 结果面板
//!
//! 布局：
//! ```text
//! ┌────────────────────────────────────┐
//! │ 1  SELECT * FROM users             │ ← code_editor("sql")
//! │ 2  WHERE id = 1                    │   line_number + folding + Tree-sitter
//! ├────────────────────────────────────┤
//! │ ▶ 运行 (⌘↵)   ⏱ 12ms · 23 行       │ ← 工具条
//! ├────────────────────────────────────┤
//! │ id │ name      │ created_at         │
//! │ ...                                 │ ← ResultPanel
//! └────────────────────────────────────┘
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_component::input::InputEvent;
use parking_lot::RwLock;

use crate::sql_completion::{SchemaCache, extract_tables_in_use_for_prefetch};

use gpui::{
    AppContext as _, ClickEvent, Context, Entity, IntoElement, ParentElement, Render, Styled, Task,
    Window, div, prelude::*, px,
};
use gpui_component::Selectable as _;
use gpui_component::{
    ActiveTheme, Disableable as _, IconName, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::Notification,
    v_flex,
};
use ramag_app::ConnectionService;
use ramag_domain::entities::{ConnectionConfig, Query};
use tracing::{error, info};

use crate::actions::{
    ExplainQuery, ExportCsv, ExportJson, ExportMarkdown, FormatSql, RunQuery,
    RunStatementAtCursor, SaveSqlFile,
};
use crate::views::result_panel::{ResultPanel, ResultState};

/// 单个查询标签
pub struct QueryTab {
    service: Arc<ConnectionService>,
    /// 当前激活的连接（None 时禁用执行）
    connection: Option<ConnectionConfig>,
    /// 当前激活的默认库；表树点击表/schema 时由父 session 同步进来。
    /// 执行 SQL 前 driver 会在同一连接上 `USE <schema>`，避免裸表名报
    /// "No database selected"
    active_schema: Option<String>,
    /// SQL 编辑器
    editor: Entity<InputState>,
    /// 结果面板
    result: Entity<ResultPanel>,
    /// 是否在执行中
    running: bool,
    /// 当前正在跑的任务句柄（drop 后取消异步任务）
    /// 注意：仅断客户端等待；要真正中断 mysql 执行还要发 KILL QUERY，见下方 handle
    current_task: Option<Task<()>>,
    /// 取消句柄：driver 在 acquire 后写入 mysql 后端 thread id（0 = 未拿到）
    /// 用户点取消按钮时读它发 `KILL QUERY <id>`
    cancel_handle: Option<ramag_domain::traits::CancelHandle>,
    /// 查询开始时间，仅 running 时为 Some，用于实时显示已耗时
    query_start: Option<Instant>,
    /// 与编辑器 / 表树共享的补全 schema 缓存（用于 DDL 后自动刷新）
    schema_cache: Arc<RwLock<SchemaCache>>,
    /// Tab 标题（默认值，如 "Query 1"）
    title: String,
    /// 上次执行的 SQL 摘要：成功执行后从 SQL 派生，作为标题展示
    short_title: Option<String>,
    /// 异步任务（如保存文件）完成后挂这里，下次 render 在 window 上推送
    pending_notification: Option<Notification>,
    /// 上游显式指定的目标表 (schema, table)：表树点击触发的 SELECT 才有
    /// 用户手动改 SQL 后再 run 时会被 set_sql 清空，避免残留误导
    pinned_target: Option<(String, String)>,
    /// 是否显示 SQL 编辑器（仅控制顶部 220px 那一块；工具条/结果表格保留）
    /// 由 QueryPanel.toggle_editor 全局同步给所有 Tab
    show_editor: bool,
    /// 自动 LIMIT 注入开关：默认 true（保护新手不误查全表）
    /// 用户也可在 SQL 内写 `-- ramag:no-limit` 单条跳过
    auto_limit_enabled: bool,
    /// 编辑器变化订阅 keep-alive
    _editor_sub: gpui::Subscription,
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
            state.lsp.completion_provider =
                Some(crate::sql_completion::SqlCompletionProvider::new_rc(cache_for_provider));
            state
        });
        let result = cx.new(|cx| {
            let mut p = ResultPanel::new(window, cx);
            // 把执行器注入：单元格编辑弹框「确认修改」需要异步发 UPDATE
            p.set_executor(Some(service.clone()), connection.clone());
            p
        });

        // 订阅编辑器内容变化：发现新提到的表 → 后台预拉它的列结构
        // 这样列名补全不必依赖用户先在表树展开该表
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
            _editor_sub: editor_sub,
        }
    }

    /// 切换自动 LIMIT 注入开关；返回切换后的状态用于日志
    pub(super) fn toggle_auto_limit(&mut self, cx: &mut Context<Self>) -> bool {
        self.auto_limit_enabled = !self.auto_limit_enabled;
        cx.notify();
        self.auto_limit_enabled
    }

    /// 当前 auto_limit 是否开启（工具条按钮高亮态需要）
    pub(super) fn auto_limit_on(&self) -> bool {
        self.auto_limit_enabled
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

    /// 扫描当前 SQL 找出 FROM / JOIN 涉及的表，对未在 cache 的表后台拉一次列结构
    /// schema 推断顺序：SQL 全限定 schema → 连接默认 database → cache.tables 反查
    fn prefetch_columns_for_used_tables(&self, cx: &mut Context<Self>) {
        let Some(conn) = self.connection.clone() else { return };
        let sql = self.editor.read(cx).value().to_string();
        let tables = extract_tables_in_use_for_prefetch(&sql);
        if tables.is_empty() {
            return;
        }

        // 把 (Option<schema>, table) 解析成 (schema, table) 对（确定的）
        let cache = self.schema_cache.clone();
        let resolved: Vec<(String, String)> = {
            let r = cache.read();
            tables
                .into_iter()
                .filter_map(|(maybe_s, t)| {
                    if let Some(s) = maybe_s {
                        return Some((s, t));
                    }
                    // active_schema（点表树同步进来）优先于连接默认 database
                    if let Some(s) = self.active_schema.clone() {
                        return Some((s, t));
                    }
                    if let Some(s) = conn.database.clone() {
                        return Some((s, t));
                    }
                    // 在 cache.tables 里反查包含该表的 schema
                    for (s, ts) in r.tables.iter() {
                        if ts.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                            return Some((s.clone(), t));
                        }
                    }
                    None
                })
                // 已 cache 过的跳过
                .filter(|(s, t)| !r.columns.contains_key(&(s.clone(), t.clone())))
                .collect()
        };
        if resolved.is_empty() {
            return;
        }

        let svc = self.service.clone();
        cx.background_spawn(async move {
            for (schema, table) in resolved {
                match svc.list_columns(&conn, &schema, &table).await {
                    Ok(cols) => {
                        let names: Vec<String> = cols.into_iter().map(|c| c.name).collect();
                        cache.write().columns.insert((schema, table), names);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "prefetch columns failed");
                    }
                }
            }
        })
        .detach();
    }

    pub fn title(&self) -> &str {
        &self.title
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

    /// 当前 active schema（UI 工具条展示）
    pub fn active_schema(&self) -> Option<&str> {
        self.active_schema.as_deref()
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
        // QueryPanel.prefill_active_sql_and_run_with_target 内 set_sql 之后再 set_pinned_target
        self.pinned_target = None;
        // set_value 不发 InputEvent::Change（emit_events=false），手动触发预拉
        // 这样表点击后立刻准备好该表的列名补全
        self.prefetch_columns_for_used_tables(cx);
        cx.notify();
    }

    /// 对外暴露：让其他视图（如点表树后）触发执行
    pub fn run(&mut self, cx: &mut Context<Self>) {
        self.handle_run(cx);
    }

    /// 聚焦编辑器（关闭 / 切换 Tab 后由 QueryPanel 调用，避免用户再点一下）
    pub fn focus_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |state, cx| {
            state.focus(window, cx);
        });
    }

    /// 取出当前编辑器中的 SQL
    fn current_sql(&self, cx: &gpui::App) -> String {
        self.editor.read(cx).value().to_string()
    }

    fn handle_run(&mut self, cx: &mut Context<Self>) {
        let sql = self.current_sql(cx);
        let trimmed = sql.trim().to_string();
        // run = 用户主动执行，标题用原 SQL 派生，DDL 后刷新 cache
        let title_sql = trimmed.clone();
        self.submit_sql(trimmed, title_sql, true, cx);
    }

    /// 仅执行光标所在的那条 SQL（按 `;` 切分；避开字符串/注释里的 `;`）
    /// 编辑器只有一条语句时，等价于 handle_run
    fn handle_run_at_cursor(&mut self, cx: &mut Context<Self>) {
        let sql = self.current_sql(cx);
        let cursor = self.editor.read(cx).cursor();
        let stmt = extract_statement_at_cursor(&sql, cursor);
        let trimmed = stmt.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        let title_sql = trimmed.clone();
        self.submit_sql(trimmed, title_sql, true, cx);
    }

    /// EXPLAIN 当前 SQL：把 SQL 包一层 `EXPLAIN ` 提交，结果展示在结果区
    /// 已经以 EXPLAIN 开头的 SQL 不重复加；末尾 `;` 自动 strip
    pub(super) fn handle_explain(&mut self, cx: &mut Context<Self>) {
        let sql = self.current_sql(cx);
        let trimmed = sql.trim().trim_end_matches(';').trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        let upper = trimmed.to_ascii_uppercase();
        let to_run = if upper.starts_with("EXPLAIN ") || upper == "EXPLAIN" {
            trimmed.clone()
        } else {
            format!("EXPLAIN {trimmed}")
        };
        // 标题用原 SQL（让 Tab 显示用户实际想看的语句，而不是 EXPLAIN xxx）
        // is_run=false：EXPLAIN 不会改 schema，跳过 DDL cache 刷新
        self.submit_sql(to_run, trimmed, false, cx);
    }

    /// 提交 SQL 到 driver：handle_run / handle_explain 共享的核心
    /// - sql_to_run: 实际发给 driver 的语句
    /// - title_sql: 用于 short_title 派生 + DDL 检测的"用户原始语句"
    /// - is_run: 是 run 还是 explain；explain 不刷新 cache
    fn submit_sql(
        &mut self,
        sql_to_run: String,
        title_sql: String,
        is_run: bool,
        cx: &mut Context<Self>,
    ) {
        if self.running {
            return;
        }
        let Some(conn) = self.connection.clone() else {
            self.result.update(cx, |r, cx| {
                r.set_state(
                    ResultState::Error("尚未选择连接".to_string()),
                    cx,
                );
            });
            return;
        };
        if sql_to_run.trim().is_empty() {
            self.result.update(cx, |r, cx| {
                r.set_state(ResultState::Error("SQL 为空".to_string()), cx);
            });
            return;
        }
        // 自动 LIMIT 注入：仅普通 run 走，且用户没在工具条关掉
        // EXPLAIN 不注入；driver 端的 Query.auto_limit 作为兜底（防止其他路径漏掉）
        let auto_limit_active = is_run && self.auto_limit_enabled;
        let sql_to_run = if auto_limit_active {
            inject_limits(&sql_to_run, AUTO_LIMIT)
        } else {
            sql_to_run
        };

        self.running = true;
        self.query_start = Some(Instant::now());
        self.result.update(cx, |r, cx| {
            r.set_state(ResultState::Running, cx);
        });
        cx.notify();

        // 后台 ticker：每 100ms notify 一次让耗时数字跳动
        // 通过 this.running 自终止，无需显式 cancel
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let still_running = this
                    .update(cx, |this, cx| {
                        if this.running {
                            cx.notify();
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                if !still_running {
                    break;
                }
            }
        })
        .detach();

        let svc = self.service.clone();
        let result_handle = self.result.clone();
        let active_schema = self.active_schema.clone();
        // 取消句柄：driver 会把 mysql 后端 thread id 写入；cancel 路径读它发 KILL QUERY
        let handle: ramag_domain::traits::CancelHandle =
            std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        self.cancel_handle = Some(handle.clone());
        // 不再 detach：保留 Task 句柄，cancel 时 drop 即可中断客户端 await
        let auto_limit_for_driver: Option<u32> = if auto_limit_active {
            // driver 兜底：万一 inject_limits 漏过（如复杂 CTE），driver 再扫一遍
            Some(AUTO_LIMIT as u32)
        } else {
            None
        };
        let task = cx.spawn(async move |this, cx| {
            let mut query = Query::new(sql_to_run).with_auto_limit(auto_limit_for_driver);
            if let Some(s) = active_schema {
                query = query.with_schema(s);
            }
            // execute_cancellable_with_history 会自动追加历史 + 把 thread id 写入 handle
            let outcome = svc
                .execute_cancellable_with_history(&conn, &query, handle)
                .await;
            let _ = this.update(cx, |this, cx| {
                this.running = false;
                this.current_task = None;
                this.cancel_handle = None;
                this.query_start = None;
                match outcome {
                    Ok(qr) => {
                        info!(
                            rows = qr.rows.len(),
                            elapsed_ms = qr.elapsed_ms,
                            "query ok"
                        );
                        // 成功时清掉之前的错误高亮
                        this.clear_sql_diagnostics(cx);
                        // 派生 tab 标题（同步给 QueryPanel TabBar）
                        this.short_title = Some(make_short_title(&title_sql));
                        // DDL 自动刷新补全 cache（CREATE / DROP / ALTER 等）；
                        // EXPLAIN 不算 DDL，跳过
                        if is_run {
                            this.maybe_refresh_cache_after_ddl(&title_sql, cx);
                        }
                        // 把当前 Tab 的 pinned_target 注入给 ResultPanel：
                        // 行内编辑优先用精确目标，不再依赖反引号 SQL parse
                        let target_for_result = this
                            .pinned_target
                            .as_ref()
                            .map(|(s, t)| (Some(s.clone()), t.clone()));
                        result_handle.update(cx, |r, cx| {
                            // 注入源 SQL 给 result_panel，让 "复制 INSERT" 能解析表名
                            r.set_source_sql(Some(title_sql.clone()));
                            r.set_pinned_target(target_for_result);
                            r.set_state(ResultState::Ok(qr), cx);
                        });
                    }
                    Err(e) => {
                        error!(error = %e, "query failed");
                        let err_msg = e.to_string();
                        // 编辑器红波浪线高亮报错位置
                        this.highlight_sql_error(&err_msg, cx);
                        result_handle.update(cx, |r, cx| {
                            r.set_state(ResultState::Error(err_msg), cx);
                        });
                    }
                }
                cx.notify();
            });
        });
        self.current_task = Some(task);
    }

    /// 检查 SQL 是否是 DDL（CREATE / DROP / ALTER / RENAME / TRUNCATE）
    /// 是的话后台拉默认 schema 的最新表名刷新 cache，让补全立刻能看到新表
    fn maybe_refresh_cache_after_ddl(&self, sql: &str, cx: &mut Context<Self>) {
        let first = sql
            .trim_start()
            .split_whitespace()
            .next()
            .map(|w| w.to_ascii_uppercase())
            .unwrap_or_default();
        let is_ddl = matches!(
            first.as_str(),
            "CREATE" | "DROP" | "ALTER" | "RENAME" | "TRUNCATE"
        );
        if !is_ddl {
            return;
        }
        let Some(conn) = self.connection.clone() else { return };
        // active_schema 优先，否则退到连接默认 database
        let Some(schema) = self
            .active_schema
            .clone()
            .or_else(|| conn.database.clone())
            .filter(|s| !s.is_empty())
        else {
            return;
        };
        let svc = self.service.clone();
        let cache = self.schema_cache.clone();
        cx.background_spawn(async move {
            match svc.list_tables(&conn, &schema).await {
                Ok(tables) => {
                    let names: Vec<String> = tables.into_iter().map(|t| t.name).collect();
                    cache.write().tables.insert(schema, names);
                    info!("schema cache refreshed after DDL");
                }
                Err(e) => {
                    error!(error = %e, "DDL refresh: list_tables failed");
                }
            }
        })
        .detach();
    }

    /// 格式化当前编辑器的 SQL（替换原内容）
    /// 空内容直接 no-op；保持当前 reindent + uppercase keywords 风格
    pub(super) fn handle_format(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sql = self.current_sql(cx);
        if sql.trim().is_empty() {
            return;
        }
        let opts = sqlformat::FormatOptions {
            indent: sqlformat::Indent::Spaces(2),
            uppercase: Some(true),
            lines_between_queries: 1,
            ignore_case_convert: None,
        };
        let formatted = sqlformat::format(&sql, &sqlformat::QueryParams::None, &opts);
        // 内容相同则不写回（避免 set_value 触发不必要的 prefetch）
        if formatted == sql {
            return;
        }
        self.editor.update(cx, |state, cx| {
            state.set_value(formatted, window, cx);
        });
        self.prefetch_columns_for_used_tables(cx);
        cx.notify();
    }

    /// 报错后在编辑器对应行加红波浪线 + 错误消息（hover 显示）
    /// MySQL error 通常带 `... at line N` —— 提取出来高亮整行
    fn highlight_sql_error(&mut self, err_msg: &str, cx: &mut Context<Self>) {
        let line_no = parse_mysql_error_line(err_msg);
        let msg_for_diag = err_msg.to_string();
        self.editor.update(cx, |state, cx| {
            // diagnostics 仅 code_editor 模式下存在
            if let Some(diag) = state.diagnostics_mut() {
                diag.clear();
                let line = line_no.unwrap_or(1).saturating_sub(1) as u32;
                let range = gpui_component::input::Position::new(line, 0)
                    ..gpui_component::input::Position::new(line, 9999);
                diag.push(
                    gpui_component::highlighter::Diagnostic::new(range, msg_for_diag)
                        .with_severity(
                            gpui_component::highlighter::DiagnosticSeverity::Error,
                        ),
                );
                cx.notify();
            }
        });
    }

    /// 清掉编辑器的错误高亮（运行成功 / 内容变化时）
    fn clear_sql_diagnostics(&mut self, cx: &mut Context<Self>) {
        self.editor.update(cx, |state, cx| {
            if let Some(diag) = state.diagnostics_mut() {
                if !diag.is_empty() {
                    diag.clear();
                    cx.notify();
                }
            }
        });
    }

    /// 保存编辑器内容为 .sql 文件：弹系统对话框，rfd 子线程，oneshot 回主
    fn handle_save_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sql = self.current_sql(cx);
        if sql.trim().is_empty() {
            window.push_notification(
                Notification::warning("SQL 为空，无需保存").autohide(true),
                cx,
            );
            return;
        }
        let default_name = format!(
            "ramag-{}.sql",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let (tx, rx) = futures::channel::oneshot::channel::<Result<std::path::PathBuf, String>>();
        std::thread::spawn(move || {
            let dialog = rfd::FileDialog::new()
                .add_filter("SQL", &["sql"])
                .set_file_name(default_name);
            let result = match dialog.save_file() {
                Some(path) => match std::fs::write(&path, sql.as_bytes()) {
                    Ok(_) => Ok(path),
                    Err(e) => Err(format!("写入失败：{e}")),
                },
                None => Err("__cancel__".to_string()),
            };
            let _ = tx.send(result);
        });

        cx.spawn(async move |this, cx| {
            let outcome = rx.await.unwrap_or_else(|_| Err("内部错误".into()));
            let _ = this.update(cx, |this, cx| match outcome {
                Ok(path) => {
                    info!(?path, "sql saved");
                    let name = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("文件")
                        .to_string();
                    this.pending_notification = Some(
                        Notification::success(format!("已保存到 {name}")).autohide(true),
                    );
                    cx.notify();
                }
                Err(e) if e == "__cancel__" => {}
                Err(e) => {
                    error!(error = %e, "save sql failed");
                    this.pending_notification = Some(
                        Notification::error(format!("保存失败：{e}")).autohide(true),
                    );
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 取消当前查询
    /// 1. drop Task 中断客户端 await
    /// 2. 若已拿到后端 thread id，detach 一个任务发 `KILL QUERY <id>` 真正中断 mysql 端语句
    fn handle_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.current_task.take().is_none() {
            return;
        }
        // 同时取出 handle，把 mysql 端的查询也终止（仅当 driver 已 store 过 thread id）
        let cancel_target = self.cancel_handle.take().and_then(|h| {
            let tid = h.load(std::sync::atomic::Ordering::SeqCst);
            if tid > 0 { Some(tid) } else { None }
        });
        if let (Some(tid), Some(conn)) = (cancel_target, self.connection.clone()) {
            let svc = self.service.clone();
            cx.spawn(async move |_this, _cx| {
                if let Err(e) = svc.cancel_query(&conn, tid).await {
                    // KILL 失败不致命：客户端已停等，最差是 mysql 端继续跑完
                    tracing::warn!(error = %e, thread_id = tid, "KILL QUERY failed");
                } else {
                    info!(thread_id = tid, "KILL QUERY sent");
                }
            })
            .detach();
        }
        self.running = false;
        self.query_start = None;
        self.result.update(cx, |r, cx| {
            r.set_state(ResultState::Empty, cx);
        });
        window.push_notification(
            Notification::info("已取消查询").autohide(true),
            cx,
        );
        info!("query cancelled");
        cx.notify();
    }
}

impl Render for QueryTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 把异步完成挂起的 toast 推送出来（如 SQL 保存结果）
        if let Some(n) = self.pending_notification.take() {
            window.push_notification(n, cx);
        }
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let border = theme.border;
        let secondary_bg = theme.secondary;
        let bg = theme.background;

        let running = self.running;
        let has_connection = self.connection.is_some();
        // 工具条不再展示 host/连接名（信息已在顶部 Tab Bar 体现，避免重复）
        // 仅保留 DB 选择器供用户切换默认库
        let schema_label = self.active_schema.clone();
        let current_schema = schema_label.clone();
        let entity_for_db = cx.entity();
        // DB 下拉读 cache 的"全量 schema 名 + 是否显示系统库"
        // 不在这里 snapshot：dropdown_menu 闭包是 Fn 每次打开时调，
        // 闭包内 cache.read() 拿当下数据，与表树眼睛 toggle 状态自动同步
        let cache_for_db = self.schema_cache.clone();

        // 仅"执行中"状态在工具条显示实时耗时，其他状态由结果面板底部 status_bar 展示
        // 避免与 status_bar 的"X 行 · 耗时 N ms"重复
        let running_elapsed = self
            .query_start
            .map(|t| t.elapsed())
            .map(format_elapsed);
        let (result_summary, has_result): (Option<String>, bool) = match self.result.read(cx).state() {
            ResultState::Ok(qr) => (None, !qr.rows.is_empty()),
            ResultState::Error(_) => (None, false),
            ResultState::Running => (
                Some(match &running_elapsed {
                    Some(s) => format!("执行中 {s}"),
                    None => "执行中".to_string(),
                }),
                false,
            ),
            ResultState::Empty => (None, false),
        };
        // 工具条「删除」按钮：多选行 OR 选中单元格 都算可删
        let panel_for_btn = self.result.read(cx);
        let has_multi_selected = !panel_for_btn.selected_rows().is_empty();
        let has_selected = has_multi_selected || panel_for_btn.selected_cell().is_some();
        let _ = panel_for_btn;

        v_flex()
            .size_full()
            .bg(bg)
            .key_context("QueryTab")
            // 监听 RunQuery action（绑定 ⌘↵ 到此 action 见 main.rs）
            .on_action(cx.listener(|this, _: &RunQuery, _, cx| {
                this.handle_run(cx);
            }))
            // ⌘⇧↵：仅执行光标所在的 SQL 语句
            .on_action(cx.listener(|this, _: &RunStatementAtCursor, _, cx| {
                this.handle_run_at_cursor(cx);
            }))
            // ExportCsv / ExportJson 通过 dropdown_menu 触发，
            // 转发给 ResultPanel 处理（它持有 QueryResult 数据）
            .on_action(cx.listener(|this, _: &ExportCsv, _, cx| {
                this.result.update(cx, |r, cx| {
                    r.export(crate::views::result_panel::ExportFormat::Csv, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &ExportJson, _, cx| {
                this.result.update(cx, |r, cx| {
                    r.export(crate::views::result_panel::ExportFormat::Json, cx);
                });
            }))
            // ⌘⇧F：格式化当前 SQL
            .on_action(cx.listener(|this, _: &FormatSql, window, cx| {
                this.handle_format(window, cx);
            }))
            // ⌘⇧E：EXPLAIN 当前 SQL
            .on_action(cx.listener(|this, _: &ExplainQuery, _, cx| {
                this.handle_explain(cx);
            }))
            // ⌘S：保存编辑器内容为 .sql 文件
            .on_action(cx.listener(|this, _: &SaveSqlFile, window, cx| {
                this.handle_save_file(window, cx);
            }))
            // SQL 编辑器：固定占上半 220px（show_editor=false 时整段不渲染，
            // 让出空间给下方结果表格；工具条和结果区始终保留）
            .when(self.show_editor, |this| {
                this.child(
                    div()
                        .h(px(220.0))
                        .flex_none()
                        .border_b_1()
                        .border_color(border)
                        .child(
                            Input::new(&self.editor)
                                .h_full()
                                .bordered(false)
                                .focus_bordered(false),
                        ),
                )
            })
            // 工具条：@ 连接名 / 结果摘要 / 导出 / 运行
            .child(
                h_flex()
                    .w_full()
                    .flex_none()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(border)
                    .bg(secondary_bg)
                    // DB 选择器：点开下拉手动切换默认库
                    // 选项 = 左侧表树/cache 已加载的 schema；点项调 set_active_schema
                    // 没有 schema 时仍渲染（提示用户先连接），避免按钮闪烁
                    .child({
                        let label = match &schema_label {
                            Some(s) => format!("DB: {s}"),
                            None => "DB: 未选库".to_string(),
                        };
                        let cur = current_schema.clone();
                        let entity = entity_for_db.clone();
                        let cache = cache_for_db.clone();
                        Button::new("schema-picker")
                            .ghost()
                            .small()
                            .label(label)
                            .dropdown_menu(move |menu, _, _| {
                                use crate::sql_completion::is_system_schema;
                                let mut menu = menu;
                                // 实时读 cache：跟表树眼睛 toggle 同步
                                let (mut opts, show_system) = {
                                    let c = cache.read();
                                    (c.all_schemas.clone(), c.show_system)
                                };
                                if !show_system {
                                    opts.retain(|s| !is_system_schema(s));
                                }
                                // 业务库优先 / 系统库置底；同组按字典序
                                opts.sort_by(|a, b| {
                                    let a_sys = is_system_schema(a);
                                    let b_sys = is_system_schema(b);
                                    a_sys.cmp(&b_sys).then_with(|| a.cmp(b))
                                });
                                if opts.is_empty() {
                                    menu = menu.label("（暂无可选库）");
                                } else {
                                    let mut last_was_business = false;
                                    let mut inserted_separator = false;
                                    for s in opts.iter() {
                                        let is_sys = is_system_schema(s);
                                        // 业务库与系统库之间插一条分隔线（仅一次）
                                        if is_sys && last_was_business && !inserted_separator {
                                            menu = menu.separator();
                                            inserted_separator = true;
                                        }
                                        last_was_business = !is_sys;
                                        let is_current =
                                            cur.as_deref() == Some(s.as_str());
                                        let entity_each = entity.clone();
                                        let s_each = s.clone();
                                        // 系统库后缀加（系统）以提示，但仍可选
                                        let label = if is_sys {
                                            format!("{s}  · 系统")
                                        } else {
                                            s.clone()
                                        };
                                        menu = menu.item(
                                            PopupMenuItem::new(label)
                                                .checked(is_current)
                                                .on_click(move |_, _, app| {
                                                    let chosen = s_each.clone();
                                                    entity_each.update(app, |this, cx| {
                                                        this.set_active_schema(
                                                            Some(chosen),
                                                            cx,
                                                        );
                                                    });
                                                }),
                                        );
                                    }
                                }
                                menu
                            })
                    })
                    // 过滤栏拆双维度：左 = 列过滤（逗号分隔多列）/ 右 = 行过滤（单关键字）
                    // 两者独立叠加；flex_1 平分中间剩余空间
                    // 列输入外包一层 div 拦截 ↑/↓：单行 Input 默认不挂 MoveUp/MoveDown
                    // handler，事件冒泡到此处后转给 InputState 的补全菜单做选项导航
                    .child({
                        let col_input = self.result.read(cx).column_filter_entity().clone();
                        let row_input = self.result.read(cx).row_filter_entity().clone();
                        let col_for_up = col_input.clone();
                        let col_for_down = col_input.clone();
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .on_action(move |action: &gpui_component::input::MoveUp, window, app| {
                                        col_for_up.update(app, |state, cx| {
                                            state.handle_action_for_context_menu(
                                                Box::new(action.clone()),
                                                window,
                                                cx,
                                            );
                                        });
                                    })
                                    .on_action(move |action: &gpui_component::input::MoveDown, window, app| {
                                        col_for_down.update(app, |state, cx| {
                                            state.handle_action_for_context_menu(
                                                Box::new(action.clone()),
                                                window,
                                                cx,
                                            );
                                        });
                                    })
                                    .child(
                                        Input::new(&col_input)
                                            .small()
                                            .bordered(false)
                                            .focus_bordered(false)
                                            .cleanable(true),
                                    ),
                            )
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(&row_input)
                                        .small()
                                        .bordered(false)
                                        .focus_bordered(false)
                                        .cleanable(true),
                                ),
                            )
                    })
                    .when_some(result_summary, |this, summary| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .child(summary),
                        )
                    })
                    // LIMIT 自动注入开关：默认开启（按钮高亮态），关闭时按钮置灰
                    // 工作机制：开 → SELECT 自动追加 LIMIT 10000；关 → 用户写啥跑啥
                    // 也支持在 SQL 写 `-- ramag:no-limit` 单条跳过
                    .child({
                        let on = self.auto_limit_on();
                        Button::new("toolbar-auto-limit")
                            .ghost()
                            .small()
                            .label("10K")
                            .selected(on)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                let now_on = this.toggle_auto_limit(cx);
                                info!(enabled = now_on, "auto limit toggled");
                            }))
                    })
                    // 新增按钮：拉表的列元数据 → 表格末尾追加可编辑草稿行（DataGrip 风格）
                    // 必须有 pinned_target（即从表树点开的单表 SELECT）
                    .child({
                        let can_insert = self.connection.is_some()
                            && self.pinned_target.is_some()
                            && self.result.read(cx).pending_insert().is_none();
                        Button::new("toolbar-insert")
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .tooltip(if can_insert {
                                "新增行"
                            } else if self.pinned_target.is_none() {
                                "新增行（请先从表树点开单表）"
                            } else {
                                "新增行（已在草稿中，先提交或取消）"
                            })
                            .disabled(!can_insert)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                let Some(conn) = this.connection.clone() else { return };
                                let Some((schema, table)) = this.pinned_target.clone()
                                else {
                                    return;
                                };
                                let svc = this.service.clone();
                                let panel = this.result.clone();
                                let handle = window.window_handle();
                                cx.spawn(async move |_, cx| {
                                    let cols = svc.list_columns(&conn, &schema, &table).await;
                                    let _ = cx.update_window(handle, |_, window, app| {
                                        match cols {
                                            Ok(cols) => {
                                                // 为每列建 InputState（草稿行的输入框）
                                                let inputs: Vec<gpui::Entity<InputState>> =
                                                    cols.iter()
                                                        .map(|col| {
                                                            let placeholder = format!(
                                                                "{} · {}",
                                                                col.data_type.raw_type,
                                                                if col.nullable { "可空" } else { "必填" }
                                                            );
                                                            app.new(|cx_inner| {
                                                                InputState::new(window, cx_inner)
                                                                    .placeholder(placeholder)
                                                            })
                                                        })
                                                        .collect();
                                                let first_input = inputs.first().cloned();
                                                panel.update(app, |r, cx| {
                                                    r.start_insert(
                                                        schema.clone(),
                                                        table.clone(),
                                                        cols,
                                                        inputs,
                                                        cx,
                                                    );
                                                });
                                                // 自动聚焦第一个输入框
                                                if let Some(input) = first_input {
                                                    input.update(app, |state, cx_inner| {
                                                        state.focus(window, cx_inner);
                                                    });
                                                }
                                            }
                                            Err(e) => {
                                                window.push_notification(
                                                    Notification::error(format!(
                                                        "拉取表结构失败：{e}"
                                                    ))
                                                    .autohide(true),
                                                    app,
                                                );
                                            }
                                        }
                                    });
                                })
                                .detach();
                            }))
                    })
                    .child(
                        // 删除按钮：多选优先（批量），无勾选则按选中单元格的行单删
                        // 都弹二次确认 dialog，确认后异步执行
                        Button::new("toolbar-delete")
                            .ghost()
                            .small()
                            .icon(IconName::Minus)
                            .tooltip("删除选中行")
                            .disabled(!has_selected)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                let panel_ref = this.result.read(cx);
                                // 多选优先；没多选才看 cell 选中
                                let multi = panel_ref.delete_preview_multi();
                                let single = if multi.is_none() {
                                    panel_ref.delete_preview()
                                } else {
                                    None
                                };
                                let _ = panel_ref;
                                let result = this.result.clone();
                                let (title, preview, on_ok_indices, on_ok_single): (
                                    &'static str,
                                    String,
                                    Option<Vec<usize>>,
                                    Option<usize>,
                                ) = match (multi, single) {
                                    (Some((ids, summary)), _) => {
                                        ("删除选中行？", summary, Some(ids), None)
                                    }
                                    (None, Some((ri, p))) => {
                                        ("删除此行？", format!("将删除：{p}"), None, Some(ri))
                                    }
                                    _ => return,
                                };
                                window.open_dialog(cx, move |dialog, _, _| {
                                    let result_btn = result.clone();
                                    let preview_for_content = preview.clone();
                                    let on_ok_indices = on_ok_indices.clone();
                                    let on_ok_single = on_ok_single;
                                    let cancel = Button::new("del-row-cancel")
                                        .ghost()
                                        .small()
                                        .label("取消")
                                        .on_click(|_: &ClickEvent, window, app| {
                                            window.close_dialog(app);
                                        });
                                    let ok = Button::new("del-row-ok")
                                        .danger()
                                        .small()
                                        .label("删除")
                                        .on_click({
                                            let result = result_btn.clone();
                                            let indices = on_ok_indices.clone();
                                            let single = on_ok_single;
                                            move |_: &ClickEvent, window, app| {
                                                result.update(app, |r, cx| {
                                                    if let Some(ids) = indices.clone() {
                                                        r.execute_delete_rows_async(ids, cx);
                                                    } else if let Some(ri) = single {
                                                        r.execute_delete_row_async(ri, cx);
                                                    }
                                                });
                                                window.close_dialog(app);
                                            }
                                        });
                                    dialog
                                        .title(title)
                                        .width(px(520.0))
                                        .margin_top(px(180.0))
                                        .content(move |c, _, cx| {
                                            let muted_fg = cx.theme().muted_foreground;
                                            let p = preview_for_content.clone();
                                            c.child(
                                                div()
                                                    .text_sm()
                                                    .text_color(muted_fg)
                                                    .child(p),
                                            )
                                        })
                                        .footer(
                                            h_flex()
                                                .w_full()
                                                .items_center()
                                                .justify_end()
                                                .gap(px(8.0))
                                                .child(cancel)
                                                .child(ok),
                                        )
                                });
                            })),
                    )
                    // 导出：CSV / JSON / Markdown 三种格式，统一弹保存对话框写文件
                    // 范围：勾选行 / 全部
                    .child(
                        Button::new("export-btn")
                            .ghost()
                            .small()
                            .icon(ramag_ui::icons::download())
                            .tooltip("导出")
                            .disabled(!has_result)
                            .dropdown_menu(|menu, _, _| {
                                menu.menu("CSV", Box::new(ExportCsv))
                                    .menu("JSON", Box::new(ExportJson))
                                    .menu("Markdown", Box::new(ExportMarkdown))
                            }),
                    )
                    // 运行中显示红色"取消"按钮，否则显示蓝色"运行"按钮（仅图标）
                    .when(running, |this| {
                        this.child(
                            Button::new("cancel-query")
                                .danger()
                                .small()
                                .icon(IconName::Close)
                                .tooltip("取消当前查询")
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.handle_cancel(window, cx);
                                })),
                        )
                    })
                    .when(!running, |this| {
                        this.child(
                            Button::new("run-query")
                                .primary()
                                .small()
                                .icon(IconName::Play)
                                .disabled(!has_connection)
                                .tooltip("⌘↵ 运行 SQL")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.handle_run(cx);
                                })),
                        )
                    }),
            )
            // 结果区：占剩余高度
            // flex_1 + min_h_0：纵向不被内容撑大；min_w_0：横向不被宽表格撑大
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(self.result.clone()),
            )
    }
}

/// 格式化运行中耗时：< 60s 显示 "X.Xs"，>= 60s 显示 "Mm Ss"
fn format_elapsed(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 60.0 {
        format!("{secs:.1}s")
    } else {
        let m = (secs / 60.0) as u64;
        let s = secs as u64 % 60;
        format!("{m}m {s}s")
    }
}

/// 默认自动 LIMIT 注入的上限
/// 提到 10000 配合表格虚拟化：服务端拉 1w 行 + 客户端 uniform_list 虚拟渲染都流畅；
/// 用户已写 LIMIT N 不会被覆盖（见 inject_limits）
/// 暴露给 connection_session 等同模块用，统一双击表名 / SHOW TABLE 等场景的 LIMIT
pub(super) const AUTO_LIMIT: usize = 10_000;

/// 给"裸 SELECT / SHOW / DESC"自动注入 LIMIT，避免误把全表拉回来。
/// 多语句时按 `;` 切分逐条处理。已经有 `LIMIT` / `WITH` / 非 SELECT 的语句保持原样。
pub(crate) fn inject_limits(sql: &str, max_rows: usize) -> String {
    let stmts = split_sql_statements(sql);
    if stmts.is_empty() {
        return sql.to_string();
    }
    let mut out = String::with_capacity(sql.len() + 16 * stmts.len());
    for (i, stmt) in stmts.iter().enumerate() {
        let s = inject_limit_one(stmt, max_rows);
        if i > 0 {
            out.push_str(";\n");
        }
        out.push_str(&s);
    }
    // 末尾分号保持（用户的写法）
    if sql.trim_end().ends_with(';') {
        out.push(';');
    }
    out
}

/// 单条语句 LIMIT 注入：仅 SELECT/WITH 类，且不含 LIMIT 时
fn inject_limit_one(stmt: &str, max_rows: usize) -> String {
    let trimmed = stmt.trim();
    if trimmed.is_empty() {
        return stmt.to_string();
    }
    let upper: String = trimmed
        .chars()
        .skip_while(|c| c.is_whitespace())
        .take(8)
        .collect::<String>()
        .to_ascii_uppercase();
    // 仅这两个开头的语句考虑（DESC/SHOW 输出小，不注入；EXPLAIN 也跳过）
    if !(upper.starts_with("SELECT") || upper.starts_with("WITH")) {
        return stmt.to_string();
    }
    // 已有 LIMIT 不重复加。简单 case-insensitive 子串匹配（FROM 子查询里有 LIMIT 也算，
    // 此时不强行加，保持用户意图）
    let upper_full = trimmed.to_ascii_uppercase();
    if has_top_level_keyword(&upper_full, "LIMIT") {
        return stmt.to_string();
    }
    // 末尾如果有分号先剔掉
    let body = trimmed.trim_end_matches(';').trim_end();
    format!("{body} LIMIT {max_rows}")
}

/// 检测 SQL 中是否有顶层（不在括号子查询里）的关键字（如 LIMIT）
/// 简化处理：跳过字符串/反引号/注释，扫描 keyword 边界
fn has_top_level_keyword(sql_upper: &str, keyword: &str) -> bool {
    let bytes = sql_upper.as_bytes();
    let kw = keyword.as_bytes();
    let mut depth: i32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\'' | b'"' | b'`' => {
                let q = b;
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == q {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
            }
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                i += 1;
            }
            _ if depth == 0 && i + kw.len() <= bytes.len() && &bytes[i..i + kw.len()] == kw => {
                let prev_ok = i == 0 || !is_ident(bytes[i - 1]);
                let next_idx = i + kw.len();
                let next_ok = next_idx >= bytes.len() || !is_ident(bytes[next_idx]);
                if prev_ok && next_ok {
                    return true;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// 同 driver 层切分语句（避开字符串/注释），但本地化避免依赖 infra-mysql
fn split_sql_statements(sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\'' | b'"' | b'`' => {
                let q = b;
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == q {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
            }
            b';' => {
                let seg = sql[start..i].trim();
                if !seg.is_empty() {
                    out.push(seg.to_string());
                }
                start = i + 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    let tail = sql[start..].trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

/// 从 MySQL 错误消息里提取 "at line N" 的行号；找不到返回 None
pub(crate) fn parse_mysql_error_line(msg: &str) -> Option<usize> {
    // 形如 "... at line 3" / "at line 12"
    let needle = " at line ";
    let idx = msg.find(needle)?;
    let tail = &msg[idx + needle.len()..];
    let num: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
    num.parse().ok()
}

/// 提取光标所在的那条 SQL 语句（按 `;` 切分）
///
/// 切分时跳过下列结构里的 `;`：
/// - 单引号 / 双引号 / 反引号 字符串（含 `\\` 转义）
/// - `--` 行注释 / `/* */` 块注释
///
/// 实现：单遍扫描 byte，记录所有 `;` 边界 → 找到包含 cursor 的边界对
/// `cursor` 是 UTF-8 byte offset；越界时按最后一条处理。
fn extract_statement_at_cursor(sql: &str, cursor: usize) -> &str {
    let bytes = sql.as_bytes();
    let cursor = cursor.min(bytes.len());
    let mut splits: Vec<usize> = Vec::new(); // `;` 自身的 byte index
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\'' | b'"' | b'`' => {
                let quote = b;
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == quote {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            b';' => {
                splits.push(i);
                i += 1;
            }
            _ => i += 1,
        }
    }

    // 按 cursor 找包含它的语句区间 [start, end)
    let mut start = 0;
    for &sp in &splits {
        if sp >= cursor {
            // 当前语句区间是 [start, sp)
            // 但 cursor 落在 sp 上时（光标紧挨 `;` 后面），用户多半是想跑这条
            // 即 sp >= cursor 时 end = sp
            return safe_str_slice(sql, start, sp);
        }
        start = sp + 1;
    }
    // cursor 在最后一个 `;` 之后（或没 `;`）→ 最后一段
    safe_str_slice(sql, start, bytes.len())
}

fn safe_str_slice(sql: &str, mut start: usize, mut end: usize) -> &str {
    let bytes = sql.as_bytes();
    // 收缩到合法 char 边界（&str slice 必须落在 char 边界）
    while start < bytes.len() && !sql.is_char_boundary(start) {
        start += 1;
    }
    while end > 0 && !sql.is_char_boundary(end) {
        end -= 1;
    }
    if end < start {
        return "";
    }
    &sql[start..end]
}

/// 从 SQL 派生短标题：取首条非空行前 28 个字符（按字符计，不按字节）
/// 超出加省略号
fn make_short_title(sql: &str) -> String {
    const MAX: usize = 28;
    let first_line = sql
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if first_line.chars().count() > MAX {
        let prefix: String = first_line.chars().take(MAX).collect();
        format!("{prefix}…")
    } else {
        first_line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::make_short_title;

    #[test]
    fn short_title_truncate() {
        assert_eq!(make_short_title("SELECT 1"), "SELECT 1");
        assert_eq!(
            make_short_title("SELECT * FROM very_long_table_name_here"),
            "SELECT * FROM very_long_tabl…"
        );
    }

    #[test]
    fn short_title_skips_blank_lines() {
        let sql = "\n\n  -- comment\nSELECT 1";
        // 首个非空行是 "-- comment"，按设计返回它
        assert_eq!(make_short_title(sql), "-- comment");
    }

    #[test]
    fn short_title_empty() {
        assert_eq!(make_short_title(""), "");
        assert_eq!(make_short_title("   "), "");
    }

    /// EXPLAIN 包装策略：模拟 handle_explain 的 SQL 处理
    fn wrap_explain(sql: &str) -> String {
        let trimmed = sql.trim().trim_end_matches(';').trim().to_string();
        if trimmed.is_empty() {
            return String::new();
        }
        let upper = trimmed.to_ascii_uppercase();
        if upper.starts_with("EXPLAIN ") || upper == "EXPLAIN" {
            trimmed
        } else {
            format!("EXPLAIN {trimmed}")
        }
    }

    #[test]
    fn parse_mysql_line() {
        assert_eq!(
            super::parse_mysql_error_line(
                "You have an error in your SQL syntax... near 'foo' at line 3"
            ),
            Some(3)
        );
        assert_eq!(super::parse_mysql_error_line("connection refused"), None);
        assert_eq!(super::parse_mysql_error_line("error at line 12"), Some(12));
    }

    #[test]
    fn inject_limit_plain_select() {
        let s = super::inject_limits("SELECT * FROM t", 1000);
        assert_eq!(s, "SELECT * FROM t LIMIT 1000");
    }

    #[test]
    fn inject_limit_skips_existing_limit() {
        let s = super::inject_limits("SELECT * FROM t LIMIT 10", 1000);
        assert_eq!(s, "SELECT * FROM t LIMIT 10");
        // 大小写不敏感
        let s = super::inject_limits("select * from t limit 10", 1000);
        assert_eq!(s, "select * from t limit 10");
    }

    #[test]
    fn inject_limit_skips_non_select() {
        // INSERT/UPDATE/DELETE/SHOW/DESC 不注入
        assert_eq!(
            super::inject_limits("UPDATE t SET a=1", 1000),
            "UPDATE t SET a=1"
        );
        assert_eq!(super::inject_limits("SHOW TABLES", 1000), "SHOW TABLES");
    }

    #[test]
    fn inject_limit_keeps_subquery_limit_alone() {
        // 子查询里的 LIMIT 不算 top-level，外层仍要注入
        let s = super::inject_limits(
            "SELECT * FROM (SELECT * FROM t LIMIT 10) x",
            1000,
        );
        assert!(s.ends_with("LIMIT 1000"));
    }

    #[test]
    fn inject_limit_strips_trailing_semicolon() {
        let s = super::inject_limits("SELECT * FROM t;", 1000);
        // 末尾 ; 保留，body 加 LIMIT
        assert_eq!(s, "SELECT * FROM t LIMIT 1000;");
    }

    #[test]
    fn extract_stmt_single() {
        // 没有分号 → 整段
        assert_eq!(
            super::extract_statement_at_cursor("SELECT 1", 5).trim(),
            "SELECT 1"
        );
    }

    #[test]
    fn extract_stmt_multi_picks_by_cursor() {
        let sql = "SELECT 1; SELECT 2; SELECT 3";
        // cursor 在 "SELECT 1" 中
        assert_eq!(super::extract_statement_at_cursor(sql, 3).trim(), "SELECT 1");
        // cursor 在 "SELECT 2" 中（位置 12 = 'L' of 2nd SELECT）
        assert_eq!(super::extract_statement_at_cursor(sql, 12).trim(), "SELECT 2");
        // cursor 在末尾 "SELECT 3"
        assert_eq!(super::extract_statement_at_cursor(sql, 25).trim(), "SELECT 3");
    }

    #[test]
    fn extract_stmt_ignores_semicolon_in_string() {
        // 字符串里的 ; 不切分
        let sql = "SELECT 'a;b'; SELECT 2";
        assert_eq!(
            super::extract_statement_at_cursor(sql, 5).trim(),
            "SELECT 'a;b'"
        );
        assert_eq!(super::extract_statement_at_cursor(sql, 18).trim(), "SELECT 2");
    }

    #[test]
    fn extract_stmt_ignores_semicolon_in_comment() {
        let sql = "SELECT 1 -- comment ;\n; SELECT 2";
        // cursor 在第一条
        let first = super::extract_statement_at_cursor(sql, 5);
        assert!(first.contains("SELECT 1"));
        assert!(!first.contains("SELECT 2"));
        // cursor 在第二条
        assert_eq!(
            super::extract_statement_at_cursor(sql, 26).trim(),
            "SELECT 2"
        );
    }

    #[test]
    fn explain_wraps_plain_select() {
        assert_eq!(wrap_explain("SELECT 1"), "EXPLAIN SELECT 1");
        assert_eq!(
            wrap_explain("SELECT * FROM t WHERE id=1;"),
            "EXPLAIN SELECT * FROM t WHERE id=1"
        );
    }

    #[test]
    fn explain_does_not_double_wrap() {
        assert_eq!(
            wrap_explain("EXPLAIN SELECT 1"),
            "EXPLAIN SELECT 1"
        );
        assert_eq!(
            wrap_explain("explain  SELECT 1"),
            "explain  SELECT 1"
        );
    }

    #[test]
    fn explain_strips_trailing_semicolons() {
        assert_eq!(
            wrap_explain("SELECT 1;;;"),
            "EXPLAIN SELECT 1"
        );
    }

    #[test]
    fn sqlformat_works() {
        // 锁定 sqlformat 的关键行为：单行 SQL → 多行缩进 + 关键字大写
        let opts = sqlformat::FormatOptions {
            indent: sqlformat::Indent::Spaces(2),
            uppercase: Some(true),
            lines_between_queries: 1,
            ignore_case_convert: None,
        };
        let formatted = sqlformat::format(
            "select id,name from users where id=1 order by name",
            &sqlformat::QueryParams::None,
            &opts,
        );
        assert!(formatted.contains("SELECT"));
        assert!(formatted.contains("FROM"));
        assert!(formatted.contains("WHERE"));
        // 多行（reindent）
        assert!(formatted.lines().count() >= 3);
    }
}
