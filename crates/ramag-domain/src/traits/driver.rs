//! Driver trait：SQL 类数据库驱动统一抽象。dyn-safe，不引入关联类型

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use async_trait::async_trait;

use crate::entities::{
    Column, ConnectionConfig, ConnectionId, ForeignKey, Index, Query, QueryResult, Schema, Table,
};
use crate::error::Result;

/// 查询取消句柄。driver 把后端 session/thread id 写入，调用方读出转交 `cancel_query`。值 0 = 还没拿到
pub type CancelHandle = Arc<AtomicU64>;

#[async_trait]
pub trait Driver: Send + Sync {
    /// 用于日志 / UI 显示，如 "mysql"
    fn name(&self) -> &'static str;

    /// 测试连接可达
    async fn test_connection(&self, config: &ConnectionConfig) -> Result<()>;

    /// 服务端版本，如 "8.0.32"
    async fn server_version(&self, _config: &ConnectionConfig) -> Result<String> {
        Err(crate::error::DomainError::NotImplemented(
            "server_version".into(),
        ))
    }

    /// 执行一条查询
    async fn execute(&self, config: &ConnectionConfig, query: &Query) -> Result<QueryResult>;

    /// 可取消版执行。默认退化到 `execute`（不支持取消）
    async fn execute_cancellable(
        &self,
        config: &ConnectionConfig,
        query: &Query,
        _handle: CancelHandle,
    ) -> Result<QueryResult> {
        self.execute(config, query).await
    }

    /// 取消正在执行的查询。MySQL 走 `KILL QUERY`，PG 走 `pg_cancel_backend`
    async fn cancel_query(&self, _config: &ConnectionConfig, _thread_id: u64) -> Result<()> {
        Err(crate::error::DomainError::NotImplemented(
            "cancel_query".into(),
        ))
    }

    async fn list_schemas(&self, config: &ConnectionConfig) -> Result<Vec<Schema>>;

    async fn list_tables(&self, config: &ConnectionConfig, schema: &str) -> Result<Vec<Table>>;

    async fn list_columns(
        &self,
        config: &ConnectionConfig,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Column>>;

    /// 含主键 / 唯一 / 普通索引
    async fn list_indexes(
        &self,
        config: &ConnectionConfig,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Index>>;

    async fn list_foreign_keys(
        &self,
        config: &ConnectionConfig,
        schema: &str,
        table: &str,
    ) -> Result<Vec<ForeignKey>>;

    /// 失效指定连接的池缓存。用户改 config 后必须调，否则会按旧 host/db 继续工作
    fn evict_pool(&self, _id: &ConnectionId) {}
}
