//! KvDriver trait：Key-Value 数据库统一抽象
//!
//! 与 [`crate::traits::Driver`] 并列 —— 后者为 SQL 类驱动（关系模型），
//! 本 trait 服务 KV 类驱动（Redis 等）。两者方法集合差异很大，强行合并
//! 会让任一侧充斥大量 NotImplemented。
//!
//! # 设计要点
//!
//! 1. dyn-safe：方法不引入关联类型；连接对象由实现内部管理（按 ConnectionId
//!    + db 索引缓存连接句柄）
//! 2. 所有方法接受 `&ConnectionConfig`，让调用方无需关心连接生命周期
//! 3. Stage 14 仅声明 L0 必备方法；Pub/Sub、MULTI、Cluster 拓扑等
//!    L1+ 能力按阶段陆续追加
//!
//! # 实现指南
//!
//! - 标准实现见 `ramag-infra-redis::RedisDriver`
//! - 连接池缓存按 `(ConnectionId, db)` 维度（Redis 的 SELECT 是连接级状态）
//! - 异步桥接复用 mysql 同款 `tokio runtime + futures oneshot` 模式

use async_trait::async_trait;

use crate::entities::{ConnectionConfig, RedisType, RedisValue, ScanResult};
use crate::error::Result;

/// KV 数据库驱动统一抽象
#[async_trait]
pub trait KvDriver: Send + Sync {
    /// 驱动名称（用于日志/UI 显示，如 "redis"）
    fn name(&self) -> &'static str;

    /// 测试连接（PING）
    async fn test_connection(&self, config: &ConnectionConfig) -> Result<()>;

    /// 服务端版本（INFO server 中的 redis_version 字段）
    async fn server_version(&self, config: &ConnectionConfig) -> Result<String>;

    /// 当前数据库 key 总数（DBSIZE）
    async fn db_size(&self, config: &ConnectionConfig, db: u8) -> Result<u64>;

    /// SCAN 分批迭代 keys
    ///
    /// - `cursor`：起始游标，第一次传 0；返回值 cursor=0 表示遍历结束
    /// - `match_pattern`：glob 风格（`user:*` / `?ession:*`）；None 不过滤
    /// - `type_filter`：仅返回指定类型 key（6.0+）；None 不过滤
    /// - `count`：建议返回数量（仅 hint，实际可多可少）；推荐 100-500
    async fn scan(
        &self,
        config: &ConnectionConfig,
        db: u8,
        cursor: u64,
        match_pattern: Option<&str>,
        type_filter: Option<RedisType>,
        count: u32,
    ) -> Result<ScanResult>;

    /// 取 key 类型（TYPE）
    async fn key_type(&self, config: &ConnectionConfig, db: u8, key: &str) -> Result<RedisType>;

    /// 取 key 剩余 TTL（毫秒，PTTL）
    /// - Some(-1): 永久（无 TTL）
    /// - Some(-2): key 不存在
    /// - Some(n>=0): 剩余毫秒
    async fn key_ttl(&self, config: &ConnectionConfig, db: u8, key: &str) -> Result<i64>;

    /// 按类型 dispatch 取 key 完整 value
    ///
    /// 内部根据 TYPE 应答自动选用：
    /// - String → GET
    /// - List → LRANGE 0 -1
    /// - Hash → HGETALL
    /// - Set → SMEMBERS
    /// - ZSet → ZRANGE 0 -1 WITHSCORES
    /// - Stream → XRANGE - +
    ///
    /// key 不存在时返回 [`RedisValue::Nil`]
    async fn get_value(&self, config: &ConnectionConfig, db: u8, key: &str) -> Result<RedisValue>;

    /// 删除 key（DEL）
    /// 返回是否真的删除了某个 key（true = 存在并删除，false = 本就不存在）
    async fn delete_key(&self, config: &ConnectionConfig, db: u8, key: &str) -> Result<bool>;

    /// 设置 / 取消 TTL
    /// - `Some(secs)`：EXPIRE key secs
    /// - `None`：PERSIST key（取消 TTL，转为永久）
    ///
    /// 返回 true 表示 key 存在且操作成功
    async fn set_ttl(
        &self,
        config: &ConnectionConfig,
        db: u8,
        key: &str,
        ttl_secs: Option<i64>,
    ) -> Result<bool>;

    /// 通用命令执行（CLI / Workbench 通道）
    ///
    /// `argv` 是命令拆分后的字符串数组，例如 `["SET", "foo", "bar"]`。
    /// 应答按 RESP 类型映射为 [`RedisValue`]，复杂应答走 Array/Hash/Map 嵌套。
    async fn execute_command(
        &self,
        config: &ConnectionConfig,
        db: u8,
        argv: Vec<String>,
    ) -> Result<RedisValue>;

    /// 取 INFO 信息（按 sections 过滤；空切片 = INFO ALL）
    ///
    /// 返回原始 INFO 文本（多 section 由 `\r\n` 分隔），由调用方解析展示
    async fn info(&self, config: &ConnectionConfig, sections: &[&str]) -> Result<String>;
}
