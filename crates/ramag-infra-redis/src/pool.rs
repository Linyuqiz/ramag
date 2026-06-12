//! Redis 连接缓存：键为 `(ConnectionId, db)`（SELECT 是连接级状态，不能跨 db 共享）。
//! ConnectionManager 自动重连 + 多路复用，clone 是 Arc 廉价复制。当前仅 standalone

use std::time::Duration;

use dashmap::DashMap;
use ramag_domain::entities::{ConnectionConfig, ConnectionId, DriverKind};
use ramag_domain::error::{DomainError, Result};
use redis::aio::ConnectionManager;
use redis::{Client, ConnectionAddr, ConnectionInfo, ProtocolVersion, RedisConnectionInfo};
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::errors::map_redis_error;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PoolKey {
    pub conn_id: ConnectionId,
    pub db: u8,
}

impl PoolKey {
    pub fn new(conn_id: ConnectionId, db: u8) -> Self {
        Self { conn_id, db }
    }
}

#[derive(Clone, Default)]
pub struct PoolCache {
    pools: Arc<DashMap<PoolKey, ConnectionManager>>,
}

impl PoolCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clone_handle(&self) -> Self {
        self.clone()
    }

    pub async fn get_or_create(
        &self,
        config: &ConnectionConfig,
        db: u8,
    ) -> Result<ConnectionManager> {
        if config.driver != DriverKind::Redis {
            return Err(DomainError::InvalidConfig(format!(
                "RedisDriver 不支持 {:?} 类型连接",
                config.driver
            )));
        }

        let key = PoolKey::new(config.id.clone(), db);

        if let Some(entry) = self.pools.get(&key) {
            debug!(connection_id = %config.id, db, "redis pool cache hit");
            return Ok(entry.clone());
        }

        info!(connection_id = %config.id, name = %config.name, host = %config.host, db, "creating redis connection manager");
        let mgr = build_connection_manager(config, db).await?;
        self.pools.insert(key, mgr.clone());
        Ok(mgr)
    }

    /// 移除该连接所有 db 的缓存（编辑配置后调）
    pub fn evict_all_dbs(&self, conn_id: &ConnectionId) {
        let to_remove: Vec<_> = self
            .pools
            .iter()
            .filter_map(|e| {
                if &e.key().conn_id == conn_id {
                    Some(e.key().clone())
                } else {
                    None
                }
            })
            .collect();
        let n = to_remove.len();
        for k in to_remove {
            self.pools.remove(&k);
        }
        if n > 0 {
            info!(connection_id = %conn_id, evicted = n, "redis pools evicted");
        }
    }
}

async fn build_connection_manager(config: &ConnectionConfig, db: u8) -> Result<ConnectionManager> {
    let info = build_connection_info(config, db);

    let client = Client::open(info).map_err(|e| {
        warn!(error = %e, host = %config.host, "build redis client failed");
        map_redis_error(e)
    })?;

    // 设连接 / 应答超时避免 GUI 卡死
    let mgr = ConnectionManager::new_with_config(
        client,
        redis::aio::ConnectionManagerConfig::new()
            .set_connection_timeout(Duration::from_secs(10))
            .set_response_timeout(Duration::from_secs(30)),
    )
    .await
    .map_err(|e| {
        warn!(error = %e, host = %config.host, "open redis connection manager failed");
        map_redis_error(e)
    })?;

    Ok(mgr)
}

/// 当前仅支持 plain TCP；TLS / Unix Socket 待扩展 ConnectionConfig schema
fn build_connection_info(config: &ConnectionConfig, db: u8) -> ConnectionInfo {
    let username = if config.username.is_empty() {
        None
    } else {
        Some(config.username.clone())
    };
    let password = if config.password.is_empty() {
        None
    } else {
        Some(config.password.clone())
    };

    ConnectionInfo {
        addr: ConnectionAddr::Tcp(config.host.clone(), config.port),
        redis: RedisConnectionInfo {
            db: db as i64,
            username,
            password,
            protocol: ProtocolVersion::RESP2,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_key_eq_and_hash() {
        let id = ConnectionId::new();
        let a = PoolKey::new(id.clone(), 0);
        let b = PoolKey::new(id.clone(), 0);
        let c = PoolKey::new(id, 1);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn build_info_no_auth() {
        let cfg = ConnectionConfig::new_redis("local", "127.0.0.1", 6379);
        let info = build_connection_info(&cfg, 0);
        assert!(matches!(info.addr, ConnectionAddr::Tcp(_, 6379)));
        assert_eq!(info.redis.db, 0);
        assert!(info.redis.username.is_none());
        assert!(info.redis.password.is_none());
    }

    #[test]
    fn build_info_with_acl() {
        let mut cfg = ConnectionConfig::new_redis("local", "127.0.0.1", 6379);
        cfg.username = "default".into();
        cfg.password = "secret".into();
        let info = build_connection_info(&cfg, 3);
        assert_eq!(info.redis.db, 3);
        assert_eq!(info.redis.username.as_deref(), Some("default"));
        assert_eq!(info.redis.password.as_deref(), Some("secret"));
    }
}
