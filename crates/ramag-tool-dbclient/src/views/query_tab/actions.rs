//! `impl QueryTab` 行为方法：运行 / 取消 / 格式化 / EXPLAIN / 错误高亮

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{AppContext as _, Context, Window};
use gpui_component::WindowExt as _;
use gpui_component::notification::Notification;
use ramag_domain::entities::Query;
use ramag_domain::error::DomainError;
use tracing::{error, info};

use super::QueryTab;
use super::paging::{Pager, page_sql, paging_base_sql};
use super::sql_utils::{
    AUTO_LIMIT, extract_statement_at_cursor, inject_limits, make_short_title,
    parse_mysql_error_line,
};
use crate::sql_completion::extract_tables_in_use_for_prefetch;
use crate::views::result_panel::ResultState;

impl QueryTab {
    /// 取出当前编辑器中的 SQL
    pub(super) fn current_sql(&self, cx: &gpui::App) -> String {
        self.editor.read(cx).value().to_string()
    }

    pub(super) fn handle_run(&mut self, cx: &mut Context<Self>) {
        let sql = self.current_sql(cx);
        let trimmed = sql.trim().to_string();
        // run = 用户主动执行，标题用原 SQL 派生，DDL 后刷新 cache
        let title_sql = trimmed.clone();
        self.submit_sql(trimmed, title_sql, true, cx);
    }

    /// 仅执行光标所在的那条 SQL（按 `;` 切分；避开字符串/注释/dollar-quoted 里的 `;`）
    pub(super) fn handle_run_at_cursor(&mut self, cx: &mut Context<Self>) {
        let sql = self.current_sql(cx);
        let cursor = self.editor.read(cx).cursor();
        let driver = self.connection.as_ref().map(|c| c.driver);
        let stmt = extract_statement_at_cursor(&sql, cursor, driver);
        let trimmed = stmt.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        let title_sql = trimmed.clone();
        self.submit_sql(trimmed, title_sql, true, cx);
    }

    /// EXPLAIN 当前 SQL：把 SQL 包一层 `EXPLAIN ` 提交，结果展示在结果区
    /// 已经以 EXPLAIN 开头的 SQL 不重复加；末尾 `;` 自动 strip
    pub(crate) fn handle_explain(&mut self, cx: &mut Context<Self>) {
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

    /// run / explain 共用核心。`sql_to_run` 实际发给 driver；`title_sql` 用于派生标题 + DDL 检测；
    /// `is_run=false` 时是 explain，不刷 schema cache
    pub(super) fn submit_sql(
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
                r.set_state(ResultState::Error("尚未选择连接".to_string()), cx);
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
        // 分页资格：注入 LIMIT 的单条裸 SELECT 记下原始语句，工具条翻页时以它重写 OFFSET
        self.pager = if auto_limit_active {
            paging_base_sql(&sql_to_run, conn.driver).map(|base_sql| Pager {
                base_sql,
                page: 0,
                has_more: false,
            })
        } else {
            None
        };
        let sql_to_run = if auto_limit_active {
            inject_limits(&sql_to_run, AUTO_LIMIT, conn.driver)
        } else {
            sql_to_run
        };
        self.execute_query(conn, sql_to_run, title_sql, is_run, auto_limit_active, cx);
    }

    /// 工具条翻页：用 pager.base_sql 重写 LIMIT/OFFSET 重跑，不重置分页状态
    pub(super) fn handle_page(&mut self, next_page: usize, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        let Some(conn) = self.connection.clone() else {
            return;
        };
        let Some(pager) = self.pager.as_mut() else {
            return;
        };
        pager.page = next_page;
        let sql = page_sql(&pager.base_sql, AUTO_LIMIT, next_page);
        let title = pager.base_sql.clone();
        self.execute_query(conn, sql, title, true, true, cx);
    }

    /// submit_sql / handle_page 共用的执行核心：状态置忙 + 后台执行 + 回调落结果
    fn execute_query(
        &mut self,
        conn: ramag_domain::entities::ConnectionConfig,
        sql_to_run: String,
        title_sql: String,
        is_run: bool,
        auto_limit_active: bool,
        cx: &mut Context<Self>,
    ) {
        self.running = true;
        self.query_start = Some(Instant::now());
        self.result.update(cx, |r, cx| {
            r.set_state(ResultState::Running, cx);
        });
        cx.notify();

        // 后台 ticker：每 100ms notify 一次让耗时数字跳动
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
        let handle: ramag_domain::traits::CancelHandle =
            Arc::new(std::sync::atomic::AtomicU64::new(0));
        self.cancel_handle = Some(handle.clone());
        let auto_limit_for_driver: Option<u32> = if auto_limit_active {
            Some(AUTO_LIMIT as u32)
        } else {
            None
        };
        let task = cx.spawn(async move |this, cx| {
            let mut query = Query::new(sql_to_run).with_auto_limit(auto_limit_for_driver);
            if let Some(s) = active_schema {
                query = query.with_schema(s);
            }
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
                        info!(rows = qr.rows.len(), elapsed_ms = qr.elapsed_ms, "query ok");
                        // 本页打满页大小 ⇒ 可能还有下一页（不跑 COUNT，按行数推断）
                        if let Some(p) = &mut this.pager {
                            p.has_more = qr.rows.len() >= AUTO_LIMIT;
                        }
                        this.clear_sql_diagnostics(cx);
                        this.short_title = Some(make_short_title(&title_sql));
                        if is_run {
                            this.maybe_refresh_cache_after_ddl(&title_sql, cx);
                        }
                        let target_for_result = this
                            .pinned_target
                            .as_ref()
                            .map(|(s, t)| (Some(s.clone()), t.clone()));
                        result_handle.update(cx, |r, cx| {
                            r.set_source_sql(Some(title_sql.clone()));
                            r.set_pinned_target(target_for_result);
                            r.set_state(ResultState::Ok(qr), cx);
                        });
                    }
                    Err(e) => {
                        error!(error = %e, "query failed");
                        let err_msg = e.to_string();
                        // 生产模式只读拦截：弹 toast 保留结果区原有内容；其余错误仍进结果区便于排查 / 复制
                        if matches!(e, DomainError::Forbidden(_)) {
                            this.pending_notification =
                                Some(Notification::warning(err_msg).autohide(true));
                        } else {
                            this.highlight_sql_error(&err_msg, cx);
                            result_handle.update(cx, |r, cx| {
                                r.set_state(ResultState::Error(err_msg), cx);
                            });
                        }
                    }
                }
                cx.notify();
            });
        });
        self.current_task = Some(task);
    }

    /// 检查 SQL 是否是 DDL（CREATE / DROP / ALTER / RENAME / TRUNCATE）
    /// 是的话后台拉默认 schema 的最新表名刷新 cache
    pub(super) fn maybe_refresh_cache_after_ddl(&self, sql: &str, cx: &mut Context<Self>) {
        let first = sql
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
        let Some(conn) = self.connection.clone() else {
            return;
        };
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
    pub(crate) fn handle_format(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    pub(super) fn highlight_sql_error(&mut self, err_msg: &str, cx: &mut Context<Self>) {
        let line_no = parse_mysql_error_line(err_msg);
        let msg_for_diag = err_msg.to_string();
        self.editor.update(cx, |state, cx| {
            if let Some(diag) = state.diagnostics_mut() {
                diag.clear();
                let line = line_no.unwrap_or(1).saturating_sub(1) as u32;
                let range = gpui_component::input::Position::new(line, 0)
                    ..gpui_component::input::Position::new(line, 9999);
                diag.push(
                    gpui_component::highlighter::Diagnostic::new(range, msg_for_diag)
                        .with_severity(gpui_component::highlighter::DiagnosticSeverity::Error),
                );
                cx.notify();
            }
        });
    }

    /// 清掉编辑器的错误高亮（运行成功 / 内容变化时）
    pub(super) fn clear_sql_diagnostics(&mut self, cx: &mut Context<Self>) {
        self.editor.update(cx, |state, cx| {
            if let Some(diag) = state.diagnostics_mut()
                && !diag.is_empty()
            {
                diag.clear();
                cx.notify();
            }
        });
    }

    /// 取消当前查询
    /// 1. drop Task 中断客户端 await
    /// 2. 若已拿到后端 thread id，detach 一个任务发 `KILL QUERY <id>` 真正中断 mysql 端语句
    pub(super) fn handle_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.current_task.take().is_none() {
            return;
        }
        let cancel_target = self.cancel_handle.take().and_then(|h| {
            let tid = h.load(std::sync::atomic::Ordering::SeqCst);
            if tid > 0 { Some(tid) } else { None }
        });
        if let (Some(tid), Some(conn)) = (cancel_target, self.connection.clone()) {
            let svc = self.service.clone();
            cx.spawn(async move |_this, _cx| {
                if let Err(e) = svc.cancel_query(&conn, tid).await {
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
        window.push_notification(Notification::info("已取消查询").autohide(true), cx);
        info!("query cancelled");
        cx.notify();
    }

    /// 扫描当前 SQL 找出 FROM / JOIN 涉及的表，对未在 cache 的表后台拉一次列结构
    /// schema 推断顺序：SQL 全限定 schema → active_schema → 连接默认 database → cache.tables 反查
    pub(super) fn prefetch_columns_for_used_tables(&self, cx: &mut Context<Self>) {
        let Some(conn) = self.connection.clone() else {
            return;
        };
        let sql = self.editor.read(cx).value().to_string();
        let tables = extract_tables_in_use_for_prefetch(&sql);
        if tables.is_empty() {
            return;
        }

        let cache = self.schema_cache.clone();
        let resolved: Vec<(String, String)> = {
            let r = cache.read();
            tables
                .into_iter()
                .filter_map(|(maybe_s, t)| {
                    if let Some(s) = maybe_s {
                        return Some((s, t));
                    }
                    if let Some(s) = self.active_schema.clone() {
                        return Some((s, t));
                    }
                    if let Some(s) = conn.database.clone() {
                        return Some((s, t));
                    }
                    for (s, ts) in r.tables.iter() {
                        if ts.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                            return Some((s.clone(), t));
                        }
                    }
                    None
                })
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
                        tracing::warn!(
                            error = %e, schema = %schema, table = %table,
                            "prefetch columns failed"
                        );
                    }
                }
            }
        })
        .detach();
    }
}
