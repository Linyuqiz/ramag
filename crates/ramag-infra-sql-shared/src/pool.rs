//! 泛型连接池缓存：按 ConnectionId 缓存 `sqlx::Pool<Db>`。DashMap + Arc 多线程安全

use std::sync::Arc;

use dashmap::DashMap;
use ramag_domain::entities::ConnectionId;
use sqlx::{Database, Pool};
use tracing::info;

pub struct PoolCache<Db: Database> {
    pools: Arc<DashMap<ConnectionId, Pool<Db>>>,
}

impl<Db: Database> Default for PoolCache<Db> {
    fn default() -> Self {
        Self {
            pools: Arc::new(DashMap::new()),
        }
    }
}

impl<Db: Database> Clone for PoolCache<Db> {
    fn clone(&self) -> Self {
        Self {
            pools: self.pools.clone(),
        }
    }
}

impl<Db: Database> PoolCache<Db> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clone_handle(&self) -> Self {
        self.clone()
    }

    /// 未命中返回 None（外部 build_pool 后 insert）
    pub fn get(&self, id: &ConnectionId) -> Option<Pool<Db>> {
        self.pools.get(id).map(|e| e.clone())
    }

    pub fn insert(&self, id: ConnectionId, pool: Pool<Db>) {
        self.pools.insert(id, pool);
    }

    pub fn evict(&self, id: &ConnectionId) {
        if self.pools.remove(id).is_some() {
            info!(connection_id = %id, "pool evicted");
        }
    }

    pub async fn close_all(&self) {
        let ids: Vec<_> = self.pools.iter().map(|e| e.key().clone()).collect();
        for id in ids {
            if let Some((_, pool)) = self.pools.remove(&id) {
                pool.close().await;
            }
        }
        info!("all pools closed");
    }

    pub fn len(&self) -> usize {
        self.pools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pools.is_empty()
    }
}
