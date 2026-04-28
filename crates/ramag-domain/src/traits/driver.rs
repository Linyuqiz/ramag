//! Driver trait：数据库驱动统一抽象
//!
//! 每种数据库（MySQL、PostgreSQL、Redis 等）在 infra 层实现自己的 Driver。
//! App 层和 UI 层只持有 `Arc<dyn Driver>`，对具体类型一无所知。

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use async_trait::async_trait;

use crate::entities::{
    Column, ConnectionConfig, ForeignKey, Index, Query, QueryResult, Schema, Table,
};
use crate::error::Result;

/// 查询取消句柄
///
/// 调用方持有 `Arc<AtomicU64>`，driver 执行查询时把后端 session/thread id 写入；
/// cancel 时调用方读出 id 转交 `cancel_query`。值为 0 = 还没拿到（无法取消）。
pub type CancelHandle = Arc<AtomicU64>;

/// 数据库驱动统一抽象
///
/// # 设计说明
///
/// 1. 使用 `async_trait` 因为返回 future 需要 trait object 兼容
/// 2. 不引入关联类型 `Conn`（那样会让 trait 不再 dyn-safe）
///    具体连接对象由实现内部管理（连接池），方法只接收 ConnectionConfig
/// 3. Stage 1 之前所有方法返回 `NotImplemented` 错误
#[async_trait]
pub trait Driver: Send + Sync {
    /// 驱动名称（用于日志/UI 显示，如 "mysql"、"postgres"）
    fn name(&self) -> &'static str;

    /// 测试连接是否可达
    async fn test_connection(&self, config: &ConnectionConfig) -> Result<()>;

    /// 获取服务端版本字符串（如 MySQL 的 "8.0.32" / "5.7.40-log"）
    ///
    /// UI 在连接列表里展示。默认返回 NotImplemented，已实现的 driver 覆盖。
    async fn server_version(&self, _config: &ConnectionConfig) -> Result<String> {
        Err(crate::error::DomainError::NotImplemented(
            "server_version".into(),
        ))
    }

    /// 执行一条查询
    async fn execute(&self, config: &ConnectionConfig, query: &Query) -> Result<QueryResult>;

    /// 可取消版执行：driver 在拿到后端 thread/session id 后写入 handle，
    /// 调用方在另一线程读出该 id 转交 `cancel_query`，即可中断本次查询。
    /// 默认实现 = `execute`（不支持取消）
    async fn execute_cancellable(
        &self,
        config: &ConnectionConfig,
        query: &Query,
        _handle: CancelHandle,
    ) -> Result<QueryResult> {
        self.execute(config, query).await
    }

    /// 取消正在执行的查询（按 thread/session id 定位）
    /// 默认未实现；MySQL 走 `KILL QUERY <id>`，PostgreSQL 走 `pg_cancel_backend(<pid>)`
    async fn cancel_query(&self, _config: &ConnectionConfig, _thread_id: u64) -> Result<()> {
        Err(crate::error::DomainError::NotImplemented(
            "cancel_query".into(),
        ))
    }

    /// 列出所有 schema（库）
    async fn list_schemas(&self, config: &ConnectionConfig) -> Result<Vec<Schema>>;

    /// 列出某个 schema 下的所有表
    async fn list_tables(&self, config: &ConnectionConfig, schema: &str) -> Result<Vec<Table>>;

    /// 列出某张表的所有列
    async fn list_columns(
        &self,
        config: &ConnectionConfig,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Column>>;

    /// 列出某张表的所有索引（含主键、唯一、普通）
    async fn list_indexes(
        &self,
        config: &ConnectionConfig,
        schema: &str,
        table: &str,
    ) -> Result<Vec<Index>>;

    /// 列出某张表的所有外键
    async fn list_foreign_keys(
        &self,
        config: &ConnectionConfig,
        schema: &str,
        table: &str,
    ) -> Result<Vec<ForeignKey>>;
}
