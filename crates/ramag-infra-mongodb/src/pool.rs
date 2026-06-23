//! MongoDB 客户端缓存：按 `ConnectionId` 缓存 `mongodb::Client`。
//! Client 内部自带连接池 + 自动重连 + 多路复用，clone 是 Arc 廉价复制；db 切换走命令而非新连接

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use mongodb::Client;
use mongodb::options::{ClientOptions, Credential, ServerAddress};
use ramag_domain::entities::{ConnectionConfig, ConnectionId, DriverKind};
use ramag_domain::error::{DomainError, Result};
use tracing::{debug, info, warn};

use crate::errors::map_mongo_error;

#[derive(Clone, Default)]
pub struct PoolCache {
    clients: Arc<DashMap<ConnectionId, Client>>,
    /// 建连串行化锁：首次打开同一连接时 prefetch_version 与 list_databases 会并发 miss，
    /// 各建一个 Client、各跑一轮 SDAM 拓扑发现，远端 prod 上表现为首开卡顿。
    /// 锁 + 双检确保每连接只建一次（再开命中缓存，无此开销）
    build_lock: Arc<tokio::sync::Mutex<()>>,
}

impl PoolCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clone_handle(&self) -> Self {
        self.clone()
    }

    pub async fn get_or_create(&self, config: &ConnectionConfig) -> Result<Client> {
        if config.driver != DriverKind::Mongodb {
            return Err(DomainError::InvalidConfig(format!(
                "MongoDriver 不支持 {:?} 类型连接",
                config.driver
            )));
        }

        if let Some(entry) = self.clients.get(&config.id) {
            debug!(connection_id = %config.id, "mongo client cache hit");
            return Ok(entry.clone());
        }

        // 串行化建连 + 双检：避免并发重复建连（各触发一轮 SDAM 发现 → 首开卡顿）
        let _guard = self.build_lock.lock().await;
        if let Some(entry) = self.clients.get(&config.id) {
            debug!(connection_id = %config.id, "mongo client cache hit (after lock)");
            return Ok(entry.clone());
        }

        info!(connection_id = %config.id, name = %config.name, host = %config.host, "creating mongo client");
        let client = build_client(config).await?;
        self.clients.insert(config.id.clone(), client.clone());
        Ok(client)
    }

    /// 移除该连接的缓存（编辑配置后调）
    pub fn evict(&self, conn_id: &ConnectionId) {
        if self.clients.remove(conn_id).is_some() {
            info!(connection_id = %conn_id, "mongo client evicted");
        }
    }

    pub fn len(&self) -> usize {
        self.clients.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }
}

async fn build_client(config: &ConnectionConfig) -> Result<Client> {
    // 用 builder 拼接 Options，避免手写 URI 时的 URL 编码陷阱
    let credential = if config.username.is_empty() {
        None
    } else {
        Some(
            Credential::builder()
                .username(Some(config.username.clone()))
                .password(Some(config.password.clone()))
                // authSource = 用户凭证所在库，独立于「浏览库」database；留空默认 admin。
                // 不再拿 database 顶替——否则指定浏览库就会把认证库指错而登不上
                .source(Some(
                    config
                        .auth_source
                        .clone()
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "admin".to_string()),
                ))
                .build(),
        )
    };

    let opts = ClientOptions::builder()
        .hosts(vec![ServerAddress::Tcp {
            host: config.host.clone(),
            port: Some(config.port),
        }])
        .credential(credential)
        .app_name(Some("ramag".to_string()))
        .connect_timeout(Some(Duration::from_secs(10)))
        .server_selection_timeout(Some(Duration::from_secs(10)))
        .build();

    let client = Client::with_options(opts).map_err(|e| {
        warn!(error = %e, host = %config.host, "build mongo client failed");
        map_mongo_error(e)
    })?;
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_cache_init_empty() {
        let cache = PoolCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn evict_nonexistent_safe() {
        let cache = PoolCache::new();
        let id = ConnectionId::new();
        // 应不报错
        cache.evict(&id);
    }
}
