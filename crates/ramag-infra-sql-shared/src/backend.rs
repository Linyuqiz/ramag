//! SqlBackend：关系型 DB 唯一抽象层
//!
//! 每个 driver crate（mysql / postgres / 未来 sqlite）只 impl 这一个 trait，
//! 通过 [`crate::impl_driver_for!`] 宏一行获得 [`ramag_domain::traits::Driver`]。
//!
//! 本模块同时提供一组泛型模板函数（test_connection_impl / execute_impl / 各 list_*_impl /
//! cancel_query_impl / server_version_impl），由宏代理调用。driver crate 不需要自己写
//! 模板逻辑，只 impl 方言方法 + per-DB 解码 + per-DB metadata SQL 即可。

use std::time::Instant;

use async_trait::async_trait;
use ramag_domain::entities::{
    Column, ConnectionConfig, ForeignKey, Index, Query, QueryResult, Row, Schema, Table, Value,
    Warning,
};
use ramag_domain::error::{DomainError, Result};
use ramag_domain::traits::CancelHandle;
use sqlx::pool::PoolConnection;
use sqlx::{Database, Executor, IntoArguments, Pool};
use tracing::{debug, info};

use crate::errors::map_sqlx_common;
use crate::pool::PoolCache;
use crate::sql::{
    SplitOptions, inject_limit_if_needed, is_query_returning_rows, split_statements,
    sql_has_no_limit_marker,
};

/// 关系型 DB 唯一抽象层
///
/// `Db` 关联到具体 sqlx Database（`sqlx::MySql` / `sqlx::Postgres` 等）。
/// 通过 [`crate::impl_driver_for!`] 宏一行获得 Domain 层 `Driver` 实现。
///
/// where 子句的 GAT HRTB 是 sqlx 0.8 必备：`Arguments<'q>` 是 GAT，要求实现者
/// 自动满足 `IntoArguments` + `&Pool/&mut PoolConnection: Executor` —— sqlx 的
/// 内置 Database 实现（MySql/Postgres/Sqlite）都满足，driver crate impl 时无感
#[async_trait]
pub trait SqlBackend: Send + Sync + 'static
where
    for<'q> <Self::Db as Database>::Arguments<'q>: IntoArguments<'q, Self::Db>,
    for<'c> &'c Pool<Self::Db>: Executor<'c, Database = Self::Db>,
    for<'c> &'c mut <Self::Db as Database>::Connection: Executor<'c, Database = Self::Db>,
{
    type Db: Database;

    fn name(&self) -> &'static str;

    /// 连接池缓存（按 ConnectionId）
    fn cache(&self) -> &PoolCache<Self::Db>;

    // === 方言（per-DB 实现）===

    /// 包裹标识符的引号字符串：MySQL 反引号 / PG 双引号
    fn quote_identifier(&self, ident: &str) -> String;

    /// 取消运行中查询的 SQL：MySQL `KILL QUERY %d` / PG `SELECT pg_cancel_backend(%d)`
    fn cancel_query_sql(&self, backend_id: u64) -> String;

    /// 切换默认库的 SQL：MySQL ``USE `db` `` / PG None（PG 必须连接时绑定 db）
    fn use_database_sql(&self, db: &str) -> Option<String>;

    /// 多语句切分选项（PG 需要识别 dollar-quoted）
    fn split_options(&self) -> SplitOptions;

    // === 连接 / 池 ===

    async fn build_pool(&self, config: &ConnectionConfig) -> Result<Pool<Self::Db>>;

    // === 行解码 ===

    fn decode_row(&self, row: &<Self::Db as Database>::Row) -> Vec<Value>;

    /// 列名 + 列类型名（per-DB 因列对象具体类型不同）
    fn extract_columns(&self, row: &<Self::Db as Database>::Row) -> (Vec<String>, Vec<String>);

    /// 空结果集 fallback 列定义（让 `SHOW`/`DESC` 等空数据语句仍能渲染列头）
    ///
    /// 默认 None：空结果集呈现为"0 行"（与 DML 一致）。
    /// MySQL 实现走 `Connection::describe` 在空结果时也能拿列定义
    async fn extract_columns_fallback(
        &self,
        _conn: &mut <Self::Db as Database>::Connection,
        _sql: &str,
    ) -> Option<(Vec<String>, Vec<String>)> {
        None
    }

    /// 取 DML 受影响行数（sqlx 没把 rows_affected 抽到 trait 上，所以靠 hook）
    fn rows_affected(&self, query_result: &<Self::Db as Database>::QueryResult) -> u64;

    /// 把后端 thread/session id 写入 cancel handle（spike 后补）
    async fn record_backend_id(
        &self,
        _conn: &mut PoolConnection<Self::Db>,
        _handle: &CancelHandle,
    ) {
    }

    /// MySQL 用来抓 SHOW WARNINGS；其他 DB 默认返回空
    async fn fetch_warnings(&self, _conn: &mut PoolConnection<Self::Db>) -> Vec<Warning> {
        Vec::new()
    }

    /// DB 错误码识别（在通用 sqlx 大类映射前优先调用）
    fn map_database_error(&self, _err: &sqlx::Error) -> Option<DomainError> {
        None
    }

    // === 元数据（per-DB SQL，但接口签名通用）===

    async fn server_version_impl(&self, pool: &Pool<Self::Db>) -> Result<String>;

    async fn list_schemas_impl(&self, pool: &Pool<Self::Db>) -> Result<Vec<Schema>>;

    async fn list_tables_impl(&self, pool: &Pool<Self::Db>, schema: &str) -> Result<Vec<Table>>;

    async fn list_columns_impl(
        &self,
        pool: &Pool<Self::Db>,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Column>>;

    async fn list_indexes_impl(
        &self,
        pool: &Pool<Self::Db>,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Index>>;

    async fn list_foreign_keys_impl(
        &self,
        pool: &Pool<Self::Db>,
        schema: &str,
        table: &str,
    ) -> Result<Vec<ForeignKey>>;
}

// === 内部工具 ===

/// 取连接池：命中缓存返回，未命中调 build_pool 后 insert
async fn get_pool<B>(b: &B, config: &ConnectionConfig) -> Result<Pool<B::Db>>
where
    B: SqlBackend,
    for<'q> <B::Db as Database>::Arguments<'q>: IntoArguments<'q, B::Db>,
    for<'c> &'c Pool<B::Db>: Executor<'c, Database = B::Db>,
    for<'c> &'c mut <B::Db as Database>::Connection: Executor<'c, Database = B::Db>,
{
    if let Some(p) = b.cache().get(&config.id) {
        return Ok(p);
    }
    let pool = b.build_pool(config).await?;
    b.cache().insert(config.id.clone(), pool.clone());
    Ok(pool)
}

/// 错误映射：先走 driver 自定义识别，未命中走 sqlx 通用大类
fn map_err<B>(b: &B, err: sqlx::Error) -> DomainError
where
    B: SqlBackend,
    for<'q> <B::Db as Database>::Arguments<'q>: IntoArguments<'q, B::Db>,
    for<'c> &'c Pool<B::Db>: Executor<'c, Database = B::Db>,
    for<'c> &'c mut <B::Db as Database>::Connection: Executor<'c, Database = B::Db>,
{
    b.map_database_error(&err)
        .unwrap_or_else(|| map_sqlx_common(&err))
}

// === 模板函数（impl_driver_for! 宏代理过来调）===

pub async fn test_connection_impl<B>(b: &B, config: &ConnectionConfig) -> Result<()>
where
    B: SqlBackend,
    for<'q> <B::Db as Database>::Arguments<'q>: IntoArguments<'q, B::Db>,
    for<'c> &'c Pool<B::Db>: Executor<'c, Database = B::Db>,
    for<'c> &'c mut <B::Db as Database>::Connection: Executor<'c, Database = B::Db>,
{
    let pool = get_pool(b, config).await?;
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .map_err(|e| map_err(b, e))?;
    Ok(())
}

pub async fn server_version_impl<B>(b: &B, config: &ConnectionConfig) -> Result<String>
where
    B: SqlBackend,
    for<'q> <B::Db as Database>::Arguments<'q>: IntoArguments<'q, B::Db>,
    for<'c> &'c Pool<B::Db>: Executor<'c, Database = B::Db>,
    for<'c> &'c mut <B::Db as Database>::Connection: Executor<'c, Database = B::Db>,
{
    let pool = get_pool(b, config).await?;
    b.server_version_impl(&pool).await
}

pub async fn list_schemas_impl<B>(b: &B, config: &ConnectionConfig) -> Result<Vec<Schema>>
where
    B: SqlBackend,
    for<'q> <B::Db as Database>::Arguments<'q>: IntoArguments<'q, B::Db>,
    for<'c> &'c Pool<B::Db>: Executor<'c, Database = B::Db>,
    for<'c> &'c mut <B::Db as Database>::Connection: Executor<'c, Database = B::Db>,
{
    let pool = get_pool(b, config).await?;
    b.list_schemas_impl(&pool).await
}

pub async fn list_tables_impl<B>(
    b: &B,
    config: &ConnectionConfig,
    schema: &str,
) -> Result<Vec<Table>>
where
    B: SqlBackend,
    for<'q> <B::Db as Database>::Arguments<'q>: IntoArguments<'q, B::Db>,
    for<'c> &'c Pool<B::Db>: Executor<'c, Database = B::Db>,
    for<'c> &'c mut <B::Db as Database>::Connection: Executor<'c, Database = B::Db>,
{
    let pool = get_pool(b, config).await?;
    b.list_tables_impl(&pool, schema).await
}

pub async fn list_columns_impl<B>(
    b: &B,
    config: &ConnectionConfig,
    schema: &str,
    table: &str,
) -> Result<Vec<Column>>
where
    B: SqlBackend,
    for<'q> <B::Db as Database>::Arguments<'q>: IntoArguments<'q, B::Db>,
    for<'c> &'c Pool<B::Db>: Executor<'c, Database = B::Db>,
    for<'c> &'c mut <B::Db as Database>::Connection: Executor<'c, Database = B::Db>,
{
    let pool = get_pool(b, config).await?;
    b.list_columns_impl(&pool, schema, table).await
}

pub async fn list_indexes_impl<B>(
    b: &B,
    config: &ConnectionConfig,
    schema: &str,
    table: &str,
) -> Result<Vec<Index>>
where
    B: SqlBackend,
    for<'q> <B::Db as Database>::Arguments<'q>: IntoArguments<'q, B::Db>,
    for<'c> &'c Pool<B::Db>: Executor<'c, Database = B::Db>,
    for<'c> &'c mut <B::Db as Database>::Connection: Executor<'c, Database = B::Db>,
{
    let pool = get_pool(b, config).await?;
    b.list_indexes_impl(&pool, schema, table).await
}

pub async fn list_foreign_keys_impl<B>(
    b: &B,
    config: &ConnectionConfig,
    schema: &str,
    table: &str,
) -> Result<Vec<ForeignKey>>
where
    B: SqlBackend,
    for<'q> <B::Db as Database>::Arguments<'q>: IntoArguments<'q, B::Db>,
    for<'c> &'c Pool<B::Db>: Executor<'c, Database = B::Db>,
    for<'c> &'c mut <B::Db as Database>::Connection: Executor<'c, Database = B::Db>,
{
    let pool = get_pool(b, config).await?;
    b.list_foreign_keys_impl(&pool, schema, table).await
}

pub async fn cancel_query_impl<B>(b: &B, config: &ConnectionConfig, backend_id: u64) -> Result<()>
where
    B: SqlBackend,
    for<'q> <B::Db as Database>::Arguments<'q>: IntoArguments<'q, B::Db>,
    for<'c> &'c Pool<B::Db>: Executor<'c, Database = B::Db>,
    for<'c> &'c mut <B::Db as Database>::Connection: Executor<'c, Database = B::Db>,
{
    let pool = get_pool(b, config).await?;
    let sql = b.cancel_query_sql(backend_id);
    sqlx::query(&sql)
        .execute(&pool)
        .await
        .map_err(|e| map_err(b, e))?;
    Ok(())
}

/// 执行查询（含多语句切分 / LIMIT 注入 / cancel handle / warnings）
pub async fn execute_impl<B>(
    b: &B,
    config: &ConnectionConfig,
    query: &Query,
    handle: Option<CancelHandle>,
) -> Result<QueryResult>
where
    B: SqlBackend,
    for<'q> <B::Db as Database>::Arguments<'q>: IntoArguments<'q, B::Db>,
    for<'c> &'c Pool<B::Db>: Executor<'c, Database = B::Db>,
    for<'c> &'c mut <B::Db as Database>::Connection: Executor<'c, Database = B::Db>,
{
    let start = Instant::now();
    let pool = get_pool(b, config).await?;
    let mut conn: PoolConnection<B::Db> = pool.acquire().await.map_err(|e| map_err(b, e))?;

    if let Some(h) = &handle {
        b.record_backend_id(&mut conn, h).await;
    }

    if let Some(schema) = query.default_schema.as_deref().filter(|s| !s.is_empty())
        && let Some(use_sql) = b.use_database_sql(schema)
    {
        debug!(?use_sql, "switching default schema before query");
        sqlx::query(&use_sql)
            .execute(&mut *conn)
            .await
            .map_err(|e| map_err(b, e))?;
    }

    let statements = split_statements(&query.sql, b.split_options());
    if statements.is_empty() {
        return Ok(QueryResult {
            columns: Vec::new(),
            column_types: Vec::new(),
            rows: Vec::new(),
            affected_rows: 0,
            elapsed_ms: start.elapsed().as_millis() as u64,
            warnings: Vec::new(),
        });
    }

    let last_idx = statements.len() - 1;
    let mut total_affected: u64 = 0;
    let mut accumulated_warnings: Vec<Warning> = Vec::new();
    let mut last_result = QueryResult {
        columns: Vec::new(),
        column_types: Vec::new(),
        rows: Vec::new(),
        affected_rows: 0,
        elapsed_ms: 0,
        warnings: Vec::new(),
    };

    let user_disabled_limit = sql_has_no_limit_marker(&query.sql);

    for (i, stmt) in statements.iter().enumerate() {
        let trimmed = stmt.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let is_select = is_query_returning_rows(trimmed);
        let injected = if is_select && !user_disabled_limit {
            inject_limit_if_needed(trimmed, query.auto_limit)
        } else {
            None
        };
        let effective_sql: &str = injected.as_deref().unwrap_or(stmt.as_str());

        let r = if is_select {
            run_select::<B>(b, &mut *conn, effective_sql).await?
        } else {
            run_dml::<B>(b, &mut *conn, effective_sql).await?
        };
        if !is_select {
            total_affected = total_affected.saturating_add(r.affected_rows);
        }
        let stmt_warnings = b.fetch_warnings(&mut conn).await;
        if !stmt_warnings.is_empty() {
            accumulated_warnings.extend(stmt_warnings);
        }
        if i == last_idx {
            last_result = r;
        }
    }

    if last_result.rows.is_empty() && last_result.columns.is_empty() {
        last_result.affected_rows = total_affected;
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;
    info!(
        elapsed_ms,
        rows = last_result.rows.len(),
        affected = last_result.affected_rows,
        statements = statements.len(),
        "query executed"
    );

    Ok(QueryResult {
        elapsed_ms,
        warnings: accumulated_warnings,
        ..last_result
    })
}

async fn run_select<B>(
    b: &B,
    conn: &mut <B::Db as Database>::Connection,
    sql: &str,
) -> Result<QueryResult>
where
    B: SqlBackend,
    for<'q> <B::Db as Database>::Arguments<'q>: IntoArguments<'q, B::Db>,
    for<'c> &'c Pool<B::Db>: Executor<'c, Database = B::Db>,
    for<'c> &'c mut <B::Db as Database>::Connection: Executor<'c, Database = B::Db>,
{
    let rows = sqlx::query(sql)
        .fetch_all(&mut *conn)
        .await
        .map_err(|e| map_err(b, e))?;

    let (columns, column_types) = if let Some(first) = rows.first() {
        b.extract_columns(first)
    } else {
        b.extract_columns_fallback(conn, sql)
            .await
            .unwrap_or_default()
    };

    let domain_rows: Vec<Row> = rows
        .iter()
        .map(|r| Row {
            values: b.decode_row(r),
        })
        .collect();

    Ok(QueryResult {
        columns,
        column_types,
        rows: domain_rows,
        affected_rows: 0,
        elapsed_ms: 0,
        warnings: Vec::new(),
    })
}

async fn run_dml<B>(
    b: &B,
    conn: &mut <B::Db as Database>::Connection,
    sql: &str,
) -> Result<QueryResult>
where
    B: SqlBackend,
    for<'q> <B::Db as Database>::Arguments<'q>: IntoArguments<'q, B::Db>,
    for<'c> &'c Pool<B::Db>: Executor<'c, Database = B::Db>,
    for<'c> &'c mut <B::Db as Database>::Connection: Executor<'c, Database = B::Db>,
{
    let result = sqlx::query(sql)
        .execute(&mut *conn)
        .await
        .map_err(|e| map_err(b, e))?;
    Ok(QueryResult {
        columns: Vec::new(),
        column_types: Vec::new(),
        rows: Vec::new(),
        affected_rows: b.rows_affected(&result),
        elapsed_ms: 0,
        warnings: Vec::new(),
    })
}
