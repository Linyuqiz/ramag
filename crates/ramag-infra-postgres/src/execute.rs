//! PG 特有 hook：抓 backend pid / 空结果列头 fallback。主执行流在 sql-shared

use std::sync::atomic::Ordering;

use ramag_domain::traits::CancelHandle;
use sqlx::postgres::PgConnection;
use sqlx::{Column as _, Executor, TypeInfo as _};
use tracing::warn;

/// 写入 backend pid 到 cancel handle（pg_cancel_backend 用）。失败仅 warn 不阻塞
pub async fn record_backend_id(conn: &mut PgConnection, handle: &CancelHandle) {
    match sqlx::query_as::<_, (i32,)>("SELECT pg_backend_pid()")
        .fetch_one(conn)
        .await
    {
        Ok((pid,)) => handle.store(pid as u64, Ordering::SeqCst),
        Err(e) => warn!(error = %e, "failed to fetch pg_backend_pid for cancel"),
    }
}

/// 空结果集 fallback：用 Executor::describe 取列头。`SELECT ... WHERE 1=0` 等会用到
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
