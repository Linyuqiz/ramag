//! MySQL 驱动。impl SqlBackend，`impl_driver_for!` 宏代理到 Driver。
//! 方言：反引号 / `KILL QUERY` / `USE <db>` / 无 dollar-quoted

pub mod errors;
pub mod execute;
pub mod metadata;
pub mod pool;
pub mod types;

use async_trait::async_trait;

use ramag_domain::entities::{
    Column, ConnectionConfig, ForeignKey, Index, Schema, Table, Value, Warning,
};
use ramag_domain::error::{DomainError, Result};
use ramag_domain::traits::CancelHandle;
use ramag_infra_sql_shared::PoolCache;
use ramag_infra_sql_shared::SqlBackend;
use ramag_infra_sql_shared::sql::SplitOptions;
use sqlx::mysql::{MySql, MySqlConnection, MySqlPool, MySqlQueryResult, MySqlRow};
use sqlx::{Column as _, Row as _, TypeInfo as _};

/// 内部仅持 Arc 包装池缓存，Clone 是 O(1)
#[derive(Clone, Default)]
pub struct MysqlDriver {
    pools: PoolCache<MySql>,
}

impl MysqlDriver {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SqlBackend for MysqlDriver {
    type Db = MySql;

    fn name(&self) -> &'static str {
        "mysql"
    }

    fn cache(&self) -> &PoolCache<Self::Db> {
        &self.pools
    }

    fn quote_identifier(&self, ident: &str) -> String {
        format!("`{}`", ident.replace('`', "``"))
    }

    fn cancel_query_sql(&self, backend_id: u64) -> String {
        format!("KILL QUERY {backend_id}")
    }

    fn use_database_sql(&self, db: &str) -> Option<String> {
        Some(format!("USE `{}`", db.replace('`', "``")))
    }

    fn split_options(&self) -> SplitOptions {
        SplitOptions::mysql()
    }

    async fn build_pool(&self, config: &ConnectionConfig) -> Result<MySqlPool> {
        pool::build_pool(config).await
    }

    fn decode_row(&self, row: &MySqlRow) -> Vec<Value> {
        types::decode_row(row)
    }

    fn extract_columns(&self, row: &MySqlRow) -> (Vec<String>, Vec<String>) {
        row.columns()
            .iter()
            .map(|c| (c.name().to_string(), c.type_info().name().to_string()))
            .unzip()
    }

    async fn extract_columns_fallback(
        &self,
        conn: &mut MySqlConnection,
        sql: &str,
    ) -> Option<(Vec<String>, Vec<String>)> {
        execute::extract_columns_fallback(conn, sql).await
    }

    fn rows_affected(&self, qr: &MySqlQueryResult) -> u64 {
        qr.rows_affected()
    }

    async fn record_backend_id(
        &self,
        conn: &mut sqlx::pool::PoolConnection<MySql>,
        handle: &CancelHandle,
    ) {
        execute::record_backend_id(conn, handle).await
    }

    async fn fetch_warnings(&self, conn: &mut sqlx::pool::PoolConnection<MySql>) -> Vec<Warning> {
        execute::fetch_warnings(conn).await
    }

    fn map_database_error(&self, err: &sqlx::Error) -> Option<DomainError> {
        errors::map_mysql_database_error(err)
    }

    async fn server_version_impl(&self, pool: &MySqlPool) -> Result<String> {
        metadata::server_version(pool).await
    }

    async fn list_schemas_impl(&self, pool: &MySqlPool) -> Result<Vec<Schema>> {
        metadata::list_schemas(pool).await
    }

    async fn list_tables_impl(&self, pool: &MySqlPool, schema: &str) -> Result<Vec<Table>> {
        metadata::list_tables(pool, schema).await
    }

    async fn list_columns_impl(
        &self,
        pool: &MySqlPool,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Column>> {
        metadata::list_columns(pool, schema, table).await
    }

    async fn list_indexes_impl(
        &self,
        pool: &MySqlPool,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Index>> {
        metadata::list_indexes(pool, schema, table).await
    }

    async fn list_foreign_keys_impl(
        &self,
        pool: &MySqlPool,
        schema: &str,
        table: &str,
    ) -> Result<Vec<ForeignKey>> {
        metadata::list_foreign_keys(pool, schema, table).await
    }
}

ramag_infra_sql_shared::impl_driver_for!(MysqlDriver);
