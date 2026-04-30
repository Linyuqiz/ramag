//! MySQL 特有的执行 hook：cancel handle 抓 thread id / SHOW WARNINGS / 空结果列定义
//!
//! 主执行流程（多语句切分 / LIMIT 注入 / cancel）由 `ramag-infra-sql-shared::execute_impl`
//! 实现；本模块提供 [`SqlBackend`](ramag_infra_sql_shared::SqlBackend) trait 上 MySQL
//! 特有 hook 的具体实现，由 lib.rs 的 trait impl 直接调用

use std::sync::atomic::Ordering;

use ramag_domain::entities::Warning;
use ramag_domain::traits::CancelHandle;
use sqlx::mysql::MySqlConnection;
use sqlx::{Column as _, Executor, TypeInfo as _};
use tracing::warn;

/// 把当前连接的 thread id 写入 cancel handle（失败仅 warn 不阻塞）
///
/// MySQL 协议中 `KILL QUERY <id>` 需要 thread id（`SELECT CONNECTION_ID()` 取）
pub async fn record_backend_id(conn: &mut MySqlConnection, handle: &CancelHandle) {
    match sqlx::query_as::<_, (u64,)>("SELECT CONNECTION_ID()")
        .fetch_one(conn)
        .await
    {
        Ok((tid,)) => handle.store(tid, Ordering::SeqCst),
        Err(e) => warn!(error = %e, "failed to fetch CONNECTION_ID for cancel"),
    }
}

/// 抓本条 statement 产生的 SHOW WARNINGS（下条语句执行会清空 buffer）
///
/// 必须在每条 statement 执行后立即调，多语句执行时由 shared 的 execute_impl 累加
pub async fn fetch_warnings(conn: &mut MySqlConnection) -> Vec<Warning> {
    use sqlx::Row as _;
    // 走 prepared statement 路径（sqlx::query 默认行为）
    //
    // 历史选择：理想做法是 sqlx::raw_sql 避开 1295 限制，但 sqlx 0.8 + async_trait
    // + tokio::spawn 组合下 HRTB 推断失败；强行做需要 unsafe transmute 违反生产原则。
    // 折衷：用 query 走 prepared，捕获 1295 静默。MySQL 8.0.14+ prepared 支持 SHOW
    // WARNINGS；老版本（含部分 8.0.x 早期）静默不报警避免日志噪音
    let rows: std::result::Result<Vec<sqlx::mysql::MySqlRow>, sqlx::Error> =
        sqlx::query("SHOW WARNINGS").fetch_all(conn).await;
    match rows {
        Ok(rows) => rows
            .into_iter()
            .filter_map(|row| {
                // SHOW WARNINGS 列固定顺序：Level / Code / Message
                let level: String = row.try_get(0).ok()?;
                let code: u32 = row.try_get(1).ok()?;
                let message: String = row.try_get(2).ok()?;
                Some(Warning {
                    level,
                    code,
                    message,
                })
            })
            .collect(),
        Err(e) => {
            // 1295 = "command not supported in prepared statement protocol"
            // 静默：服务器版本限制，不影响主流程
            let is_unsupported =
                e.as_database_error().and_then(|d| d.code()).as_deref() == Some("1295");
            if !is_unsupported {
                warn!(error = %e, "fetch SHOW WARNINGS failed (non-fatal)");
            }
            Vec::new()
        }
    }
}

/// 空结果集 fallback：走 `Connection::describe` 拿 SHOW/DESC 等无数据语句的列定义
///
/// MySQL 行为：SHOW TABLES 在 schema 没表时返回空结果集，但有列定义。
/// 不走 fallback 会让 UI 误判为 DML，渲染成"0 行受影响"
pub async fn extract_columns_fallback(
    conn: &mut MySqlConnection,
    sql: &str,
) -> Option<(Vec<String>, Vec<String>)> {
    match conn.describe(sql).await {
        Ok(desc) => Some(
            desc.columns
                .iter()
                .map(|c| (c.name().to_string(), c.type_info().name().to_string()))
                .unzip(),
        ),
        Err(e) => {
            warn!(error = %e, "describe empty-result SQL failed (non-fatal)");
            None
        }
    }
}
