//! KvDriver trait：KV 类数据库统一抽象。与 SQL Driver 并列。
//! dyn-safe；连接池按 `(ConnectionId, db)` 缓存（SELECT 是连接级状态）

use async_trait::async_trait;

use crate::entities::{ConnectionConfig, ConnectionId, RedisType, RedisValue, ScanResult};
use crate::error::Result;

#[async_trait]
pub trait KvDriver: Send + Sync {
    /// 用于日志 / UI 显示，如 "redis"
    fn name(&self) -> &'static str;

    /// PING
    async fn test_connection(&self, config: &ConnectionConfig) -> Result<()>;

    /// INFO server 的 redis_version
    async fn server_version(&self, config: &ConnectionConfig) -> Result<String>;

    /// DBSIZE
    async fn db_size(&self, config: &ConnectionConfig, db: u8) -> Result<u64>;

    /// SCAN 分批迭代。`cursor`=0 起、返回 0 终；`count` 推荐 100-500（仅 hint）
    async fn scan(
        &self,
        config: &ConnectionConfig,
        db: u8,
        cursor: u64,
        match_pattern: Option<&str>,
        type_filter: Option<RedisType>,
        count: u32,
    ) -> Result<ScanResult>;

    async fn key_type(&self, config: &ConnectionConfig, db: u8, key: &str) -> Result<RedisType>;

    /// PTTL：-1=永久，-2=key 不存在，>=0=剩余毫秒
    async fn key_ttl(&self, config: &ConnectionConfig, db: u8, key: &str) -> Result<i64>;

    /// 按 TYPE dispatch 取完整 value（GET / LRANGE / HGETALL / SMEMBERS / ZRANGE WITHSCORES / XRANGE）
    /// key 不存在返回 [`RedisValue::Nil`]
    async fn get_value(&self, config: &ConnectionConfig, db: u8, key: &str) -> Result<RedisValue>;

    /// DEL。true=删除了 key，false=本就不存在
    async fn delete_key(&self, config: &ConnectionConfig, db: u8, key: &str) -> Result<bool>;

    /// Some(secs)=EXPIRE，None=PERSIST。返回 true 表示 key 存在且成功
    async fn set_ttl(
        &self,
        config: &ConnectionConfig,
        db: u8,
        key: &str,
        ttl_secs: Option<i64>,
    ) -> Result<bool>;

    /// 通用命令执行。argv 拆分后的命令数组，应答按 RESP 类型映射 [`RedisValue`]
    async fn execute_command(
        &self,
        config: &ConnectionConfig,
        db: u8,
        argv: Vec<String>,
    ) -> Result<RedisValue>;

    /// INFO，sections 空切片 = INFO ALL。返回原始文本
    async fn info(&self, config: &ConnectionConfig, sections: &[&str]) -> Result<String>;

    /// 失效指定连接的池缓存。用户改 config 后必须调
    fn evict_pool(&self, _id: &ConnectionId) {}
}
