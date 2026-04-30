//! ConnectionService：连接管理用例聚合（多 driver 分发）
//!
//! 把 SQL 类 Driver（MySQL / PostgreSQL / 未来 SQLite）和 Storage 组合成上层 UI
//! 友好的 API。UI 只持有 `Arc<ConnectionService>`，不需要知道具体 Driver 实现。
//!
//! # 多 driver 分发
//!
//! 内部持有 `HashMap<DriverKind, Arc<dyn Driver>>`，按 `config.driver` 路由到
//! 对应 driver。Redis 走独立的 `RedisService`，不在本 service 内。

use std::collections::HashMap;
use std::sync::Arc;

use ramag_domain::entities::{
    Column, ConnectionConfig, ConnectionId, DriverKind, ForeignKey, Index, Query, QueryRecord,
    QueryRecordId, QueryResult, Schema, Table,
};
use ramag_domain::error::{DomainError, Result};
use ramag_domain::traits::{CancelHandle, Driver, Storage};

/// 连接管理服务（SQL 类多 driver 分发）
pub struct ConnectionService {
    /// 按 [`DriverKind`] 分发到具体 driver 实现
    drivers: HashMap<DriverKind, Arc<dyn Driver>>,
    storage: Arc<dyn Storage>,
}

impl ConnectionService {
    /// 创建实例
    ///
    /// `drivers` 至少应包含一个 driver；按 `config.driver` 选 driver 实例。
    /// 找不到对应 driver 时各方法返回 `DomainError::InvalidConfig`
    pub fn new(drivers: HashMap<DriverKind, Arc<dyn Driver>>, storage: Arc<dyn Storage>) -> Self {
        Self { drivers, storage }
    }

    /// 按 config.driver 取对应 driver；找不到返回 InvalidConfig
    fn driver_for(&self, config: &ConnectionConfig) -> Result<&Arc<dyn Driver>> {
        self.drivers
            .get(&config.driver)
            .ok_or_else(|| DomainError::InvalidConfig(format!("驱动不可用: {:?}", config.driver)))
    }

    // === 连接配置 CRUD（走 storage，无需 driver 路由）===

    /// 列出所有保存的连接（含 MySQL / Postgres / Redis 等所有 driver）
    pub async fn list(&self) -> Result<Vec<ConnectionConfig>> {
        self.storage.list_connections().await
    }

    pub async fn get(&self, id: &ConnectionId) -> Result<Option<ConnectionConfig>> {
        self.storage.get_connection(id).await
    }

    pub async fn save(&self, config: &ConnectionConfig) -> Result<()> {
        self.storage.save_connection(config).await
    }

    pub async fn delete(&self, id: &ConnectionId) -> Result<()> {
        self.storage.delete_connection(id).await
    }

    // === 连接动作（走 driver）===

    pub async fn test(&self, config: &ConnectionConfig) -> Result<()> {
        self.driver_for(config)?.test_connection(config).await
    }

    pub async fn server_version(&self, config: &ConnectionConfig) -> Result<String> {
        self.driver_for(config)?.server_version(config).await
    }

    /// 测试 + 保存（一键操作）
    pub async fn test_and_save(&self, config: &ConnectionConfig) -> Result<()> {
        self.driver_for(config)?.test_connection(config).await?;
        self.storage.save_connection(config).await?;
        Ok(())
    }

    // === 元数据查询（走 driver）===

    pub async fn list_schemas(&self, config: &ConnectionConfig) -> Result<Vec<Schema>> {
        self.driver_for(config)?.list_schemas(config).await
    }

    pub async fn list_tables(&self, config: &ConnectionConfig, schema: &str) -> Result<Vec<Table>> {
        self.driver_for(config)?.list_tables(config, schema).await
    }

    pub async fn list_columns(
        &self,
        config: &ConnectionConfig,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Column>> {
        self.driver_for(config)?
            .list_columns(config, schema, table)
            .await
    }

    pub async fn list_indexes(
        &self,
        config: &ConnectionConfig,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Index>> {
        self.driver_for(config)?
            .list_indexes(config, schema, table)
            .await
    }

    pub async fn list_foreign_keys(
        &self,
        config: &ConnectionConfig,
        schema: &str,
        table: &str,
    ) -> Result<Vec<ForeignKey>> {
        self.driver_for(config)?
            .list_foreign_keys(config, schema, table)
            .await
    }

    // === 查询执行 ===

    pub async fn execute(&self, config: &ConnectionConfig, query: &Query) -> Result<QueryResult> {
        self.driver_for(config)?.execute(config, query).await
    }

    pub async fn cancel_query(&self, config: &ConnectionConfig, thread_id: u64) -> Result<()> {
        self.driver_for(config)?
            .cancel_query(config, thread_id)
            .await
    }

    /// 可取消版「带历史」执行：driver 把后端 thread id 写入 handle，
    /// 上层 UI 在另一线程读 handle → 调 `cancel_query` 中断本次查询
    pub async fn execute_cancellable_with_history(
        &self,
        config: &ConnectionConfig,
        query: &Query,
        handle: CancelHandle,
    ) -> Result<QueryResult> {
        let result = match self.driver_for(config) {
            Ok(driver) => driver.execute_cancellable(config, query, handle).await,
            Err(e) => Err(e),
        };
        self.append_history_for(config, query, &result).await;
        result
    }

    pub async fn execute_with_history(
        &self,
        config: &ConnectionConfig,
        query: &Query,
    ) -> Result<QueryResult> {
        let result = match self.driver_for(config) {
            Ok(driver) => driver.execute(config, query).await,
            Err(e) => Err(e),
        };
        self.append_history_for(config, query, &result).await;
        result
    }

    /// 把 query 结果（成功/失败）追加到历史；写历史失败仅 warn 不阻塞
    async fn append_history_for(
        &self,
        config: &ConnectionConfig,
        query: &Query,
        result: &Result<QueryResult>,
    ) {
        let record = match result {
            Ok(r) => QueryRecord::new_success(
                config.id.clone(),
                config.name.clone(),
                query.sql.clone(),
                r.elapsed_ms,
                if r.rows.is_empty() {
                    r.affected_rows
                } else {
                    r.rows.len() as u64
                },
            ),
            Err(e) => QueryRecord::new_failed(
                config.id.clone(),
                config.name.clone(),
                query.sql.clone(),
                e.to_string(),
            ),
        };
        if let Err(e) = self.storage.append_history(&record).await {
            tracing::warn!(error = %e, "append history failed");
        }
    }

    // === 查询历史（走 storage）===

    pub async fn list_history(
        &self,
        connection_id: Option<&ConnectionId>,
        limit: usize,
    ) -> Result<Vec<QueryRecord>> {
        self.storage.list_history(connection_id, limit).await
    }

    pub async fn delete_history(&self, id: &QueryRecordId) -> Result<()> {
        self.storage.delete_history(id).await
    }

    pub async fn clear_history(&self, connection_id: Option<&ConnectionId>) -> Result<()> {
        self.storage.clear_history(connection_id).await
    }
}
