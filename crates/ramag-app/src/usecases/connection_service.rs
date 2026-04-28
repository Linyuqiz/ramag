//! ConnectionService：连接管理用例聚合
//!
//! 把 Driver 和 Storage 组合成上层 UI 友好的 API。UI 只持有
//! Arc<ConnectionService>，不需要知道 Driver/Storage 的具体类型。

use std::sync::Arc;

use ramag_domain::entities::{
    Column, ConnectionConfig, ConnectionId, ForeignKey, Index, Query, QueryRecord, QueryRecordId,
    QueryResult, Schema, Table,
};
use ramag_domain::error::Result;
use ramag_domain::traits::{CancelHandle, Driver, Storage};

/// 连接管理服务
///
/// 聚合所有"连接相关"的业务用例。
pub struct ConnectionService {
    driver: Arc<dyn Driver>,
    storage: Arc<dyn Storage>,
}

impl ConnectionService {
    pub fn new(driver: Arc<dyn Driver>, storage: Arc<dyn Storage>) -> Self {
        Self { driver, storage }
    }

    // === 连接配置 CRUD ===

    /// 列出所有保存的连接（含 MySQL / Redis 等所有 driver）
    ///
    /// dbclient 工具是统一连接管理入口，列表展示所有 driver；
    /// 调用方按 [`DriverKind`] 字段决定打开方式（SQL 编辑器 / Key 树 等）
    pub async fn list(&self) -> Result<Vec<ConnectionConfig>> {
        self.storage.list_connections().await
    }

    /// 按 ID 取连接
    pub async fn get(&self, id: &ConnectionId) -> Result<Option<ConnectionConfig>> {
        self.storage.get_connection(id).await
    }

    /// 保存（新增或更新）
    pub async fn save(&self, config: &ConnectionConfig) -> Result<()> {
        self.storage.save_connection(config).await
    }

    /// 删除
    pub async fn delete(&self, id: &ConnectionId) -> Result<()> {
        self.storage.delete_connection(id).await
    }

    // === 连接动作 ===

    /// 测试连接
    pub async fn test(&self, config: &ConnectionConfig) -> Result<()> {
        self.driver.test_connection(config).await
    }

    /// 取服务端版本字符串（如 MySQL "8.0.32"）
    pub async fn server_version(&self, config: &ConnectionConfig) -> Result<String> {
        self.driver.server_version(config).await
    }

    /// 测试 + 保存（一键操作）
    pub async fn test_and_save(&self, config: &ConnectionConfig) -> Result<()> {
        self.driver.test_connection(config).await?;
        self.storage.save_connection(config).await?;
        Ok(())
    }

    // === 元数据查询 ===

    pub async fn list_schemas(&self, config: &ConnectionConfig) -> Result<Vec<Schema>> {
        self.driver.list_schemas(config).await
    }

    pub async fn list_tables(&self, config: &ConnectionConfig, schema: &str) -> Result<Vec<Table>> {
        self.driver.list_tables(config, schema).await
    }

    pub async fn list_columns(
        &self,
        config: &ConnectionConfig,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Column>> {
        self.driver.list_columns(config, schema, table).await
    }

    pub async fn list_indexes(
        &self,
        config: &ConnectionConfig,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Index>> {
        self.driver.list_indexes(config, schema, table).await
    }

    pub async fn list_foreign_keys(
        &self,
        config: &ConnectionConfig,
        schema: &str,
        table: &str,
    ) -> Result<Vec<ForeignKey>> {
        self.driver.list_foreign_keys(config, schema, table).await
    }

    // === 查询执行 ===

    pub async fn execute(&self, config: &ConnectionConfig, query: &Query) -> Result<QueryResult> {
        self.driver.execute(config, query).await
    }

    /// 取消正在运行的查询（按后端 thread/session id 定位）
    pub async fn cancel_query(&self, config: &ConnectionConfig, thread_id: u64) -> Result<()> {
        self.driver.cancel_query(config, thread_id).await
    }

    /// 可取消版「带历史」执行：driver 把后端 thread id 写入 handle，
    /// 上层 UI 在另一线程读 handle → 调 `cancel_query` 中断本次查询
    pub async fn execute_cancellable_with_history(
        &self,
        config: &ConnectionConfig,
        query: &Query,
        handle: CancelHandle,
    ) -> Result<QueryResult> {
        let result = self.driver.execute_cancellable(config, query, handle).await;
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

    /// 执行查询并自动写入历史记录
    pub async fn execute_with_history(
        &self,
        config: &ConnectionConfig,
        query: &Query,
    ) -> Result<QueryResult> {
        let result = self.driver.execute(config, query).await;
        let record = match &result {
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
        // 历史写失败不影响主流程，仅 warn
        if let Err(e) = self.storage.append_history(&record).await {
            tracing::warn!(error = %e, "append history failed");
        }
        result
    }

    // === 查询历史 ===

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
