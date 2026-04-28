//! Ramag MySQL 驱动实现
//!
//! 实现 [`ramag_domain::traits::Driver`]，封装 sqlx::MySqlPool。
//!
//! # 设计要点
//!
//! - **连接池缓存**：按 `ConnectionId` 缓存 `MySqlPool`，避免每次重建
//! - **双 runtime 桥接**：通过 [`runtime::run_in_tokio`] 把 sqlx 调用派发到 tokio
//! - **类型映射**：覆盖 13+ MySQL 类型 → Domain `Value`（[`types::decode_row`]）
//! - **错误转换**：MySQL 错误码 → 中文友好提示（[`errors::map_sqlx_error`]）
//!
//! # 用法
//!
//! ```no_run
//! use std::sync::Arc;
//! use ramag_domain::traits::Driver;
//! use ramag_domain::entities::ConnectionConfig;
//! use ramag_infra_mysql::MysqlDriver;
//!
//! # async fn demo() -> ramag_domain::error::Result<()> {
//! let driver: Arc<dyn Driver> = Arc::new(MysqlDriver::new());
//! let config = ConnectionConfig::new_mysql("local", "127.0.0.1", 3306, "root");
//! driver.test_connection(&config).await?;
//! # Ok(()) }
//! ```

pub mod errors;
pub mod execute;
pub mod metadata;
pub mod pool;
pub mod runtime;
pub mod types;

use async_trait::async_trait;

use ramag_domain::entities::{
    Column, ConnectionConfig, ForeignKey, Index, Query, QueryResult, Schema, Table,
};
use ramag_domain::error::Result;
use ramag_domain::traits::{CancelHandle, Driver};

use crate::pool::PoolCache;
use crate::runtime::run_in_tokio;

/// MySQL 驱动
///
/// 内部持有连接池缓存，多线程安全（DashMap）。
pub struct MysqlDriver {
    pools: PoolCache,
}

impl MysqlDriver {
    pub fn new() -> Self {
        Self {
            pools: PoolCache::new(),
        }
    }

    /// 显式关闭所有连接池（程序退出前调用，可选）
    pub async fn shutdown(&self) {
        self.pools.close_all().await;
    }

    /// 配置变更后调用，强制下次重建连接池
    pub fn evict_pool(&self, id: &ramag_domain::entities::ConnectionId) {
        self.pools.evict(id);
    }
}

impl Default for MysqlDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Driver for MysqlDriver {
    fn name(&self) -> &'static str {
        "mysql"
    }

    async fn test_connection(&self, config: &ConnectionConfig) -> Result<()> {
        let config = config.clone();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let pool = pools.get_or_create(&config).await?;
            metadata::ping(&pool).await
        })
        .await
    }

    async fn server_version(&self, config: &ConnectionConfig) -> Result<String> {
        let config = config.clone();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let pool = pools.get_or_create(&config).await?;
            metadata::server_version(&pool).await
        })
        .await
    }

    async fn execute(&self, config: &ConnectionConfig, query: &Query) -> Result<QueryResult> {
        let config = config.clone();
        let query = query.clone();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let pool = pools.get_or_create(&config).await?;
            execute::execute(&pool, &query, None).await
        })
        .await
    }

    async fn execute_cancellable(
        &self,
        config: &ConnectionConfig,
        query: &Query,
        handle: CancelHandle,
    ) -> Result<QueryResult> {
        let config = config.clone();
        let query = query.clone();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let pool = pools.get_or_create(&config).await?;
            execute::execute(&pool, &query, Some(handle)).await
        })
        .await
    }

    async fn cancel_query(&self, config: &ConnectionConfig, thread_id: u64) -> Result<()> {
        let config = config.clone();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let pool = pools.get_or_create(&config).await?;
            execute::kill_query(&pool, thread_id).await
        })
        .await
    }

    async fn list_schemas(&self, config: &ConnectionConfig) -> Result<Vec<Schema>> {
        let config = config.clone();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let pool = pools.get_or_create(&config).await?;
            metadata::list_schemas(&pool).await
        })
        .await
    }

    async fn list_tables(&self, config: &ConnectionConfig, schema: &str) -> Result<Vec<Table>> {
        let config = config.clone();
        let schema = schema.to_string();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let pool = pools.get_or_create(&config).await?;
            metadata::list_tables(&pool, &schema).await
        })
        .await
    }

    async fn list_columns(
        &self,
        config: &ConnectionConfig,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Column>> {
        let config = config.clone();
        let schema = schema.to_string();
        let table = table.to_string();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let pool = pools.get_or_create(&config).await?;
            metadata::list_columns(&pool, &schema, &table).await
        })
        .await
    }

    async fn list_indexes(
        &self,
        config: &ConnectionConfig,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Index>> {
        let config = config.clone();
        let schema = schema.to_string();
        let table = table.to_string();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let pool = pools.get_or_create(&config).await?;
            metadata::list_indexes(&pool, &schema, &table).await
        })
        .await
    }

    async fn list_foreign_keys(
        &self,
        config: &ConnectionConfig,
        schema: &str,
        table: &str,
    ) -> Result<Vec<ForeignKey>> {
        let config = config.clone();
        let schema = schema.to_string();
        let table = table.to_string();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let pool = pools.get_or_create(&config).await?;
            metadata::list_foreign_keys(&pool, &schema, &table).await
        })
        .await
    }
}
