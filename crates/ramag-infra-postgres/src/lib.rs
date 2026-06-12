//! PostgreSQL 驱动。impl SqlBackend，`impl_driver_for!` 宏代理到 Driver。
//! 方言：双引号 / `pg_cancel_backend()` / 连接级 db / 切默认 schema 走 SET search_path / 识别 dollar-quoted

pub mod errors;
pub mod execute;
pub mod metadata;
pub mod pool;
pub mod types;

use async_trait::async_trait;

use ramag_domain::entities::{Column, ConnectionConfig, ForeignKey, Index, Schema, Table, Value};
use ramag_domain::error::{DomainError, Result};
use ramag_domain::traits::CancelHandle;
use ramag_infra_sql_shared::PoolCache;
use ramag_infra_sql_shared::SqlBackend;
use ramag_infra_sql_shared::sql::SplitOptions;
use sqlx::postgres::{PgPool, PgQueryResult, PgRow, Postgres};
use sqlx::{Column as _, Row as _, TypeInfo as _};

/// 内部仅持 Arc 包装池缓存，Clone 是 O(1)
#[derive(Clone, Default)]
pub struct PostgresDriver {
    pools: PoolCache<Postgres>,
}

impl PostgresDriver {
    pub fn new() -> Self {
        Self::default()
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
        // PG 库是连接级，无法切；这里发 SET search_path 让裸表名按选定 schema 解析（对齐 MySQL UX）
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
