//! RedisService：Redis 连接 + KV 操作聚合，与 ConnectionService 并列。
//! Storage 与 ConnectionService 共用同一份 redb，list() 按 driver 过滤互不污染

use std::sync::Arc;

use ramag_domain::entities::{
    ConnectionConfig, ConnectionId, DriverKind, KeyMeta, RedisType, RedisValue, ScanResult,
};
use ramag_domain::error::Result;
use ramag_domain::traits::{KvDriver, Storage};

pub struct RedisService {
    driver: Arc<dyn KvDriver>,
    storage: Arc<dyn Storage>,
}

impl RedisService {
    pub fn new(driver: Arc<dyn KvDriver>, storage: Arc<dyn Storage>) -> Self {
        Self { driver, storage }
    }

    // 连接 CRUD（仅 Redis driver 的连接）

    /// 按 driver 过滤
    pub async fn list(&self) -> Result<Vec<ConnectionConfig>> {
        let all = self.storage.list_connections().await?;
        Ok(all
            .into_iter()
            .filter(|c| matches!(c.driver, DriverKind::Redis))
            .collect())
    }

    /// 不限制 driver；调用方自检
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

    /// 失效该连接所有 db 的池
    pub fn evict_pool(&self, id: &ConnectionId) {
        self.driver.evict_pool(id);
    }

    pub async fn test_and_save(&self, config: &ConnectionConfig) -> Result<()> {
        self.driver.test_connection(config).await?;
        self.storage.save_connection(config).await?;
        Ok(())
    }

    // KV 操作（按 db 索引）

    pub async fn db_size(&self, config: &ConnectionConfig, db: u8) -> Result<u64> {
        self.driver.db_size(config, db).await
    }

    pub async fn scan(
        &self,
        config: &ConnectionConfig,
        db: u8,
        cursor: u64,
        pattern: Option<&str>,
        type_filter: Option<RedisType>,
        count: u32,
    ) -> Result<ScanResult> {
        self.driver
            .scan(config, db, cursor, pattern, type_filter, count)
            .await
    }

    /// 一次性扫完整库；大库慎用
    pub async fn scan_all(
        &self,
        config: &ConnectionConfig,
        db: u8,
        pattern: Option<&str>,
        type_filter: Option<RedisType>,
        max_keys: usize,
    ) -> Result<Vec<KeyMeta>> {
        let mut cursor = 0u64;
        let mut out: Vec<KeyMeta> = Vec::new();
        loop {
            let r = self
                .driver
                .scan(config, db, cursor, pattern, type_filter, 200)
                .await?;
            out.extend(r.keys);
            cursor = r.cursor;
            if cursor == 0 || out.len() >= max_keys {
                break;
            }
        }
        if out.len() > max_keys {
            out.truncate(max_keys);
        }
        Ok(out)
    }

    pub async fn key_type(
        &self,
        config: &ConnectionConfig,
        db: u8,
        key: &str,
    ) -> Result<RedisType> {
        self.driver.key_type(config, db, key).await
    }

    pub async fn key_ttl(&self, config: &ConnectionConfig, db: u8, key: &str) -> Result<i64> {
        self.driver.key_ttl(config, db, key).await
    }

    pub async fn get_value(
        &self,
        config: &ConnectionConfig,
        db: u8,
        key: &str,
    ) -> Result<RedisValue> {
        self.driver.get_value(config, db, key).await
    }

    pub async fn delete_key(&self, config: &ConnectionConfig, db: u8, key: &str) -> Result<bool> {
        self.driver.delete_key(config, db, key).await
    }

    pub async fn set_ttl(
        &self,
        config: &ConnectionConfig,
        db: u8,
        key: &str,
        ttl_secs: Option<i64>,
    ) -> Result<bool> {
        self.driver.set_ttl(config, db, key, ttl_secs).await
    }

    pub async fn execute_command(
        &self,
        config: &ConnectionConfig,
        db: u8,
        argv: Vec<String>,
    ) -> Result<RedisValue> {
        self.driver.execute_command(config, db, argv).await
    }

    pub async fn info(&self, config: &ConnectionConfig, sections: &[&str]) -> Result<String> {
        self.driver.info(config, sections).await
    }
}
