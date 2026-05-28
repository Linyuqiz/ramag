//! DocDriver trait：文档数据库（MongoDB / 未来 CouchDB / DynamoDB）统一抽象。
//! 与 Driver（SQL）/ KvDriver（KV）/ GitDriver 并列；dyn-safe，不引入关联类型

use async_trait::async_trait;

use crate::entities::{
    ConnectionConfig, ConnectionId, MongoCollection, MongoCollectionStats, MongoDatabase,
    MongoDocument, MongoIndex, MongoQueryResult, MongoQuerySpec,
};
use crate::error::Result;

#[async_trait]
pub trait DocDriver: Send + Sync {
    /// 用于日志 / UI 显示，如 "mongodb"
    fn name(&self) -> &'static str;

    /// 连通性探活（mongo 走 `ping` 命令）
    async fn test_connection(&self, config: &ConnectionConfig) -> Result<()>;

    /// 服务端版本（`buildInfo.version`）
    async fn server_version(&self, config: &ConnectionConfig) -> Result<String>;

    // 元数据

    async fn list_databases(&self, config: &ConnectionConfig) -> Result<Vec<MongoDatabase>>;

    async fn list_collections(
        &self,
        config: &ConnectionConfig,
        db: &str,
    ) -> Result<Vec<MongoCollection>>;

    async fn list_indexes(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
    ) -> Result<Vec<MongoIndex>>;

    async fn collection_stats(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
    ) -> Result<MongoCollectionStats>;

    // 查询

    async fn find(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        spec: &MongoQuerySpec,
    ) -> Result<MongoQueryResult>;

    async fn count(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        filter: &MongoDocument,
    ) -> Result<u64>;

    async fn aggregate(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        pipeline: Vec<MongoDocument>,
    ) -> Result<MongoQueryResult>;

    // 写操作

    async fn insert_one(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        document: MongoDocument,
    ) -> Result<String>;

    async fn update_one(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        filter: &MongoDocument,
        update: &MongoDocument,
    ) -> Result<MongoQueryResult>;

    async fn delete_one(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        filter: &MongoDocument,
    ) -> Result<MongoQueryResult>;

    /// 兜底通用命令。任何未抽象的 db command（dbStats / serverStatus / createIndex 等）
    async fn run_command(
        &self,
        config: &ConnectionConfig,
        db: &str,
        command: MongoDocument,
    ) -> Result<MongoDocument>;

    /// 失效指定连接的池缓存。用户改 config 后必须调
    fn evict_pool(&self, _id: &ConnectionId) {}
}
