//! MongoService：MongoDB 连接 + 文档操作聚合，与 ConnectionService / RedisService 并列。
//! Storage 与 ConnectionService 共用同一份 redb，list() 按 driver 过滤互不污染

use std::sync::Arc;

use ramag_domain::entities::{
    ConnectionConfig, ConnectionId, DriverKind, MongoCollection, MongoCollectionStats,
    MongoDatabase, MongoDocument, MongoIndex, MongoQueryResult, MongoQuerySpec, QueryRecord,
};
use ramag_domain::error::Result;
use ramag_domain::traits::{DocDriver, Storage};

pub struct MongoService {
    driver: Arc<dyn DocDriver>,
    storage: Arc<dyn Storage>,
}

impl MongoService {
    pub fn new(driver: Arc<dyn DocDriver>, storage: Arc<dyn Storage>) -> Self {
        Self { driver, storage }
    }

    // 连接 CRUD（仅 Mongo driver 的连接）

    /// 按 driver 过滤
    pub async fn list(&self) -> Result<Vec<ConnectionConfig>> {
        let all = self.storage.list_connections().await?;
        Ok(all
            .into_iter()
            .filter(|c| matches!(c.driver, DriverKind::Mongodb))
            .collect())
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

    // 连接动作

    pub async fn test(&self, config: &ConnectionConfig) -> Result<()> {
        self.driver.test_connection(config).await
    }

    pub async fn server_version(&self, config: &ConnectionConfig) -> Result<String> {
        self.driver.server_version(config).await
    }

    pub fn evict_pool(&self, id: &ConnectionId) {
        self.driver.evict_pool(id);
    }

    pub async fn test_and_save(&self, config: &ConnectionConfig) -> Result<()> {
        self.driver.test_connection(config).await?;
        self.storage.save_connection(config).await?;
        Ok(())
    }

    // 元数据

    pub async fn list_databases(&self, config: &ConnectionConfig) -> Result<Vec<MongoDatabase>> {
        self.driver.list_databases(config).await
    }

    pub async fn list_collections(
        &self,
        config: &ConnectionConfig,
        db: &str,
    ) -> Result<Vec<MongoCollection>> {
        self.driver.list_collections(config, db).await
    }

    pub async fn list_indexes(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
    ) -> Result<Vec<MongoIndex>> {
        self.driver.list_indexes(config, db, coll).await
    }

    pub async fn collection_stats(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
    ) -> Result<MongoCollectionStats> {
        self.driver.collection_stats(config, db, coll).await
    }

    // 查询

    pub async fn find(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        spec: &MongoQuerySpec,
    ) -> Result<MongoQueryResult> {
        self.driver.find(config, db, coll, spec).await
    }

    pub async fn count(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        filter: &MongoDocument,
    ) -> Result<u64> {
        self.driver.count(config, db, coll, filter).await
    }

    pub async fn aggregate(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        pipeline: Vec<MongoDocument>,
    ) -> Result<MongoQueryResult> {
        self.driver.aggregate(config, db, coll, pipeline).await
    }

    // 写

    pub async fn insert_one(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        document: MongoDocument,
    ) -> Result<String> {
        self.driver.insert_one(config, db, coll, document).await
    }

    pub async fn update_one(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        filter: &MongoDocument,
        update: &MongoDocument,
    ) -> Result<MongoQueryResult> {
        self.driver
            .update_one(config, db, coll, filter, update)
            .await
    }

    pub async fn delete_one(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        filter: &MongoDocument,
    ) -> Result<MongoQueryResult> {
        self.driver.delete_one(config, db, coll, filter).await
    }

    pub async fn run_command(
        &self,
        config: &ConnectionConfig,
        db: &str,
        command: MongoDocument,
    ) -> Result<MongoDocument> {
        self.driver.run_command(config, db, command).await
    }

    // 查询历史：与 SQL 类共用同一张 redb 表，sql 字段存原始 JSON 命令
    // 这样切换 driver 后查询历史面板能统一展示

    pub async fn append_history(
        &self,
        config: &ConnectionConfig,
        command_text: String,
        result: &Result<MongoQueryResult>,
    ) {
        let record = match result {
            Ok(r) => {
                let rows = if r.documents.is_empty() {
                    r.affected
                } else {
                    r.documents.len() as u64
                };
                QueryRecord::new_success(
                    config.id.clone(),
                    config.name.clone(),
                    command_text,
                    r.elapsed_ms,
                    rows,
                )
            }
            Err(e) => QueryRecord::new_failed(
                config.id.clone(),
                config.name.clone(),
                command_text,
                e.to_string(),
            ),
        };
        if let Err(e) = self.storage.append_history(&record).await {
            tracing::warn!(error = %e, "append mongo history failed");
        }
    }
}
