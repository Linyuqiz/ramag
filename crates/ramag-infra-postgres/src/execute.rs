//! PostgreSQL 特有的执行 hook：cancel handle 抓 backend pid + 空结果集列头 fallback
//!
//! 主执行流程（多语句切分含 dollar-quoted / LIMIT 注入 / cancel）由
//! `ramag-infra-sql-shared::execute_impl` 实现；本模块提供 [`SqlBackend`] trait 上 PG
//! 特有 hook 的实现，由 lib.rs 的 trait impl 直接调用

use std::sync::atomic::Ordering;

use ramag_domain::traits::CancelHandle;
use sqlx::postgres::PgConnection;
use sqlx::{Column as _, Executor, TypeInfo as _};
use tracing::warn;

/// 把当前连接的 backend pid 写入 cancel handle（失败仅 warn 不阻塞）
///
/// PG 协议中 `pg_cancel_backend(<pid>)` 需要 backend pid（`SELECT pg_backend_pid()` 取）。
/// pg_backend_pid 返回 i32（PG 进程号），转 u64 存入 handle
pub async fn record_backend_id(conn: &mut PgConnection, handle: &CancelHandle) {
    match sqlx::query_as::<_, (i32,)>("SELECT pg_backend_pid()")
        .fetch_one(conn)
        .await
    {
        Ok((pid,)) => handle.store(pid as u64, Ordering::SeqCst),
        Err(e) => warn!(error = %e, "failed to fetch pg_backend_pid for cancel"),
    }
}

/// 空结果集 fallback：走 sqlx Executor::describe 拿列定义
///
/// 用户跑 `SELECT * FROM t WHERE 1=0` 等语句时返回空 rows 但有列头；
/// 不走 fallback 会让 UI 误判为 DML，渲染成"0 行受影响"。与 MySQL 行为对齐
pub async fn extract_columns_fallback(
    conn: &mut PgConnection,
    sql: &str,
) -> Option<(Vec<String>, Vec<String>)> {
    match (&mut *conn).describe(sql).await {
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
