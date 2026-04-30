//! Ramag PostgreSQL 驱动
//!
//! 实现 [`ramag_infra_sql_shared::SqlBackend`] trait，由 `impl_driver_for!` 宏一行
//! 自动获得 [`ramag_domain::traits::Driver`] 实现。
//!
//! # 设计要点
//!
//! - **唯一抽象层**：只 impl `SqlBackend` 一个 trait，与 MySQL crate 完全对称
//! - **连接池缓存**：复用 `sql-shared::PoolCache<Postgres>`（按 ConnectionId 缓存）
//! - **方言**：双引号标识符 / `pg_cancel_backend(pid)` 取消 / 必须连接时绑定 db /
//!   多语句切分识别 `$$ ... $$` dollar-quoted

pub mod errors;
pub mod execute;
pub mod metadata;
pub mod pool;
pub mod types;

use async_trait::async_trait;

use ramag_domain::entities::{
    Column, ConnectionConfig, ConnectionId, ForeignKey, Index, Schema, Table, Value,
};
use ramag_domain::error::{DomainError, Result};
use ramag_domain::traits::CancelHandle;
use ramag_infra_sql_shared::PoolCache;
use ramag_infra_sql_shared::SqlBackend;
use ramag_infra_sql_shared::sql::SplitOptions;
use sqlx::postgres::{PgPool, PgQueryResult, PgRow, Postgres};
use sqlx::{Column as _, Row as _, TypeInfo as _};

/// PostgreSQL driver
///
/// 内部仅持有 `Arc` 包装的连接池缓存；Clone 是 O(1) 引用计数 +1，
/// 满足 [`impl_driver_for!`](ramag_infra_sql_shared::impl_driver_for) 对 Clone 的要求
#[derive(Clone, Default)]
pub struct PostgresDriver {
    pools: PoolCache<Postgres>,
}

impl PostgresDriver {
    pub fn new() -> Self {
        Self::default()
    }

    /// 配置变更后调用，强制下次重建连接池
    pub fn evict_pool(&self, id: &ConnectionId) {
        self.pools.evict(id);
    }

    /// 显式关闭所有池（程序退出前调用）
    pub async fn shutdown(&self) {
        self.pools.close_all().await;
    }
}

#[async_trait]
impl SqlBackend for PostgresDriver {
    type Db = Postgres;

    fn name(&self) -> &'static str {
        "postgres"
    }

    fn cache(&self) -> &PoolCache<Self::Db> {
        &self.pools
    }

    fn quote_identifier(&self, ident: &str) -> String {
        format!("\"{}\"", ident.replace('"', "\"\""))
    }

    fn cancel_query_sql(&self, backend_id: u64) -> String {
        format!("SELECT pg_cancel_backend({backend_id})")
    }

    fn use_database_sql(&self, db: &str) -> Option<String> {
        // PG 不能像 MySQL 那样 USE 切库（PG 库是连接级），但可以切默认 schema
        // 解析顺序——MySQL 双击 schema 节点会发 USE，PG 这里改发 SET search_path
        // 让裸表名 SQL 解析到该 schema（与 MySQL 体验对齐）
        Some(format!(
            "SET search_path TO \"{}\"",
            db.replace('"', "\"\"")
        ))
    }

    fn split_options(&self) -> SplitOptions {
        SplitOptions::postgres()
    }

    async fn build_pool(&self, config: &ConnectionConfig) -> Result<PgPool> {
        pool::build_pool(config).await
    }

    fn decode_row(&self, row: &PgRow) -> Vec<Value> {
        types::decode_row(row)
    }

    fn extract_columns(&self, row: &PgRow) -> (Vec<String>, Vec<String>) {
        row.columns()
            .iter()
            .map(|c| (c.name().to_string(), c.type_info().name().to_string()))
            .unzip()
    }

    async fn extract_columns_fallback(
        &self,
        conn: &mut sqlx::postgres::PgConnection,
        sql: &str,
    ) -> Option<(Vec<String>, Vec<String>)> {
        execute::extract_columns_fallback(conn, sql).await
    }

    fn rows_affected(&self, qr: &PgQueryResult) -> u64 {
        qr.rows_affected()
    }

    async fn record_backend_id(
        &self,
        conn: &mut sqlx::pool::PoolConnection<Postgres>,
        handle: &CancelHandle,
    ) {
        execute::record_backend_id(conn, handle).await
    }

    fn map_database_error(&self, err: &sqlx::Error) -> Option<DomainError> {
        errors::map_postgres_database_error(err)
    }

    async fn server_version_impl(&self, pool: &PgPool) -> Result<String> {
        metadata::server_version(pool).await
    }

    async fn list_schemas_impl(&self, pool: &PgPool) -> Result<Vec<Schema>> {
        metadata::list_schemas(pool).await
    }

    async fn list_tables_impl(&self, pool: &PgPool, schema: &str) -> Result<Vec<Table>> {
        metadata::list_tables(pool, schema).await
    }

    async fn list_columns_impl(
        &self,
        pool: &PgPool,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Column>> {
        metadata::list_columns(pool, schema, table).await
    }

    async fn list_indexes_impl(
        &self,
        pool: &PgPool,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Index>> {
        metadata::list_indexes(pool, schema, table).await
    }

    async fn list_foreign_keys_impl(
        &self,
        pool: &PgPool,
        schema: &str,
        table: &str,
    ) -> Result<Vec<ForeignKey>> {
        metadata::list_foreign_keys(pool, schema, table).await
    }
}

ramag_infra_sql_shared::impl_driver_for!(PostgresDriver);
