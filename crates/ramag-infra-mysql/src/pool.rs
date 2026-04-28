//! MySQL 连接池管理
//!
//! 按 ConnectionConfig 缓存 sqlx::MySqlPool。
//!
//! # 缓存策略
//!
//! 用 ConnectionId 作为 key（每个连接配置有唯一 UUID）。
//! 同一连接的多次查询复用同一连接池；连接配置变更（编辑后保存）应该
//! 通过 `evict` 移除旧池后重建。
//!
//! 池本身由 sqlx 管理底层连接（最大连接数、空闲回收、健康检查）。

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use ramag_domain::entities::{ConnectionConfig, ConnectionId, DriverKind};
use ramag_domain::error::{DomainError, Result};
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions, MySqlSslMode};
use sqlx::{ConnectOptions, MySqlPool};
use tracing::{debug, info, warn};

/// MySQL 连接池缓存
///
/// 多线程安全（DashMap 内部分桶锁），通过 Arc 共享所有权，
/// 调用 `clone_handle()` 把缓存的句柄移入异步闭包不会复制底层数据。
#[derive(Clone, Default)]
pub struct PoolCache {
    pools: Arc<DashMap<ConnectionId, MySqlPool>>,
}

impl PoolCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// 复制一个共享的句柄（Arc clone，O(1)）
    pub fn clone_handle(&self) -> Self {
        self.clone()
    }

    /// 获取或创建对应配置的连接池
    pub async fn get_or_create(&self, config: &ConnectionConfig) -> Result<MySqlPool> {
        if config.driver != DriverKind::Mysql {
            return Err(DomainError::InvalidConfig(format!(
                "MysqlDriver 不支持 {:?} 类型连接",
                config.driver
            )));
        }

        if let Some(entry) = self.pools.get(&config.id) {
            debug!(connection_id = %config.id, "pool cache hit");
            return Ok(entry.clone());
        }

        info!(connection_id = %config.id, name = %config.name, host = %config.host, "creating new pool");
        let pool = build_pool(config).await?;
        self.pools.insert(config.id.clone(), pool.clone());
        Ok(pool)
    }

    /// 移除某个配置的连接池（编辑配置后调用，强制下次重建）
    pub fn evict(&self, id: &ConnectionId) {
        if self.pools.remove(id).is_some() {
            info!(connection_id = %id, "pool evicted");
        }
    }

    /// 关闭所有池（程序退出时调用）
    pub async fn close_all(&self) {
        let ids: Vec<_> = self.pools.iter().map(|e| e.key().clone()).collect();
        for id in ids {
            if let Some((_, pool)) = self.pools.remove(&id) {
                pool.close().await;
            }
        }
        info!("all pools closed");
    }

    /// 当前缓存的连接池数量（调试用）
    pub fn len(&self) -> usize {
        self.pools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pools.is_empty()
    }
}

/// 用 ConnectionConfig 构建一个 sqlx::MySqlPool
async fn build_pool(config: &ConnectionConfig) -> Result<MySqlPool> {
    let opts = MySqlConnectOptions::new()
        .host(&config.host)
        .port(config.port)
        .username(&config.username)
        .password(&config.password)
        // utf8mb4 支持 emoji 和所有中文，**强制要求**
        .charset("utf8mb4")
        // 默认时区 UTC，避免歧义
        .timezone(Some("+00:00".into()))
        // SSL 默认偏好（有则用、无则降级），后续 Stage 9 加 UI 可选
        .ssl_mode(MySqlSslMode::Preferred)
        // 关掉每条 SQL 的 INFO 日志，太吵
        .log_statements(tracing::log::LevelFilter::Debug)
        .log_slow_statements(tracing::log::LevelFilter::Warn, Duration::from_secs(1));

    let opts = if let Some(db) = config.database.as_ref().filter(|s| !s.is_empty()) {
        opts.database(db)
    } else {
        opts
    };

    let pool = MySqlPoolOptions::new()
        .max_connections(8)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Some(Duration::from_secs(60 * 5)))
        .test_before_acquire(true)
        .connect_with(opts)
        .await
        .map_err(|e| {
            warn!(error = %e, host = %config.host, "build mysql pool failed");
            crate::errors::map_sqlx_error(e)
        })?;

    Ok(pool)
}
