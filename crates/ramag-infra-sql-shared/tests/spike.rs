//! Spike 验证：写一个最小 stub MysqlBackend impl SqlBackend，
//! 确认具体类型 `sqlx::MySql` 能满足 trait 的所有 GAT HRTB bound，
//! 且调用泛型模板函数 `test_connection_impl` 等能编译

use async_trait::async_trait;
use ramag_domain::entities::{
    Column, ConnectionConfig, ForeignKey, Index, Schema, Table, Value, Warning,
};
use ramag_domain::error::Result;
use ramag_infra_sql_shared::sql::SplitOptions;
use ramag_infra_sql_shared::{PoolCache, SqlBackend};
use sqlx::pool::PoolConnection;
use sqlx::{MySql, Pool};

#[derive(Clone)]
struct StubMysqlBackend {
    pools: PoolCache<MySql>,
}

ramag_infra_sql_shared::impl_driver_for!(StubMysqlBackend);

#[async_trait]
impl SqlBackend for StubMysqlBackend {
    type Db = MySql;

    fn name(&self) -> &'static str {
        "mysql-stub"
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

    async fn build_pool(&self, _config: &ConnectionConfig) -> Result<Pool<Self::Db>> {
        unimplemented!("stub: 不真实建连，只为验证类型系统")
    }

    fn decode_row(&self, _row: &<Self::Db as sqlx::Database>::Row) -> Vec<Value> {
        Vec::new()
    }

    fn extract_columns(
        &self,
        _row: &<Self::Db as sqlx::Database>::Row,
    ) -> (Vec<String>, Vec<String>) {
        (Vec::new(), Vec::new())
    }

    fn rows_affected(&self, _qr: &<Self::Db as sqlx::Database>::QueryResult) -> u64 {
        0
    }

    async fn server_version_impl(&self, _pool: &Pool<Self::Db>) -> Result<String> {
        Ok("stub".into())
    }

    async fn list_schemas_impl(&self, _pool: &Pool<Self::Db>) -> Result<Vec<Schema>> {
        Ok(Vec::new())
    }

    async fn list_tables_impl(&self, _pool: &Pool<Self::Db>, _schema: &str) -> Result<Vec<Table>> {
        Ok(Vec::new())
    }

    async fn list_columns_impl(
        &self,
        _pool: &Pool<Self::Db>,
        _schema: &str,
        _table: &str,
    ) -> Result<Vec<Column>> {
        Ok(Vec::new())
    }

    async fn list_indexes_impl(
        &self,
        _pool: &Pool<Self::Db>,
        _schema: &str,
        _table: &str,
    ) -> Result<Vec<Index>> {
        Ok(Vec::new())
    }

    async fn list_foreign_keys_impl(
        &self,
        _pool: &Pool<Self::Db>,
        _schema: &str,
        _table: &str,
    ) -> Result<Vec<ForeignKey>> {
        Ok(Vec::new())
    }

    async fn fetch_warnings(&self, _conn: &mut PoolConnection<Self::Db>) -> Vec<Warning> {
        Vec::new()
    }
}

/// 仅验证类型闭合：所有泛型模板函数能在具体 B = StubMysqlBackend 下编译
#[allow(dead_code)]
fn type_check_template_functions() {
    let backend = StubMysqlBackend {
        pools: PoolCache::new(),
    };
    let config = ConnectionConfig::new_mysql("dummy", "127.0.0.1", 3306, "root");

    // 所有泛型函数引用一遍，确认 bound 能闭合
    let _f1 = ramag_infra_sql_shared::test_connection_impl(&backend, &config);
    let _f2 = ramag_infra_sql_shared::server_version_impl(&backend, &config);
    let _f3 = ramag_infra_sql_shared::list_schemas_impl(&backend, &config);
    let _f4 = ramag_infra_sql_shared::list_tables_impl(&backend, &config, "x");
    let _f5 = ramag_infra_sql_shared::list_columns_impl(&backend, &config, "x", "y");
    let _f6 = ramag_infra_sql_shared::list_indexes_impl(&backend, &config, "x", "y");
    let _f7 = ramag_infra_sql_shared::list_foreign_keys_impl(&backend, &config, "x", "y");
}
