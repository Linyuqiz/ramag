//! Storage trait：本地持久化统一抽象
//!
//! 用于存储用户的连接配置、查询历史、收藏夹等。
//! Infra 层用 redb 实现，但 trait 设计与具体存储引擎无关。

use async_trait::async_trait;

use crate::entities::{ConnectionConfig, ConnectionId, QueryRecord, QueryRecordId};
use crate::error::Result;

/// 本地存储统一抽象
#[async_trait]
pub trait Storage: Send + Sync {
    // === 连接配置 ===

    /// 列出所有保存的连接配置
    async fn list_connections(&self) -> Result<Vec<ConnectionConfig>>;

    /// 根据 ID 获取连接配置
    async fn get_connection(&self, id: &ConnectionId) -> Result<Option<ConnectionConfig>>;

    /// 保存（新增或更新）一个连接配置
    async fn save_connection(&self, config: &ConnectionConfig) -> Result<()>;

    /// 删除一个连接配置
    async fn delete_connection(&self, id: &ConnectionId) -> Result<()>;

    // === 查询历史 ===

    /// 追加一条查询记录
    async fn append_history(&self, record: &QueryRecord) -> Result<()>;

    /// 列出查询历史（按 executed_at desc 排序）
    ///
    /// - `connection_id`: None=全部连接的历史，Some=仅指定连接
    /// - `limit`: 返回最大条数
    async fn list_history(
        &self,
        connection_id: Option<&ConnectionId>,
        limit: usize,
    ) -> Result<Vec<QueryRecord>>;

    /// 删除单条历史
    async fn delete_history(&self, id: &QueryRecordId) -> Result<()>;

    /// 清空指定连接的历史（None=清空全部）
    async fn clear_history(&self, connection_id: Option<&ConnectionId>) -> Result<()>;

    // === 通用偏好 KV ===

    /// 读取一个偏好字符串（不存在返回 None）
    async fn get_preference(&self, key: &str) -> Result<Option<String>>;

    /// 写入一个偏好字符串
    async fn set_preference(&self, key: &str, value: &str) -> Result<()>;
}
