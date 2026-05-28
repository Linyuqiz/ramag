//! MongoDriver。impl DocDriver。每个方法 clone config + pool 句柄 → run_in_tokio → dispatch 到 metadata / query

use async_trait::async_trait;
use ramag_domain::entities::{
    ConnectionConfig, ConnectionId, MongoCollection, MongoCollectionStats, MongoDatabase,
    MongoDocument, MongoIndex, MongoQueryResult, MongoQuerySpec,
};
use ramag_domain::error::Result;
use ramag_domain::traits::DocDriver;

use crate::metadata;
use crate::pool::PoolCache;
use crate::query;
use crate::runtime::run_in_tokio;

pub struct MongoDriver {
    pools: PoolCache,
}

impl MongoDriver {
    pub fn new() -> Self {
        Self {
            pools: PoolCache::new(),
        }
    }
}

impl Default for MongoDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DocDriver for MongoDriver {
    fn name(&self) -> &'static str {
        "mongodb"
    }

    async fn test_connection(&self, config: &ConnectionConfig) -> Result<()> {
        let config = config.clone();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let client = pools.get_or_create(&config).await?;
            query::ping(&client).await
        })
        .await
    }

    async fn server_version(&self, config: &ConnectionConfig) -> Result<String> {
        let config = config.clone();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let client = pools.get_or_create(&config).await?;
            query::server_version(&client).await
        })
        .await
    }

    async fn list_databases(&self, config: &ConnectionConfig) -> Result<Vec<MongoDatabase>> {
        let config = config.clone();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let client = pools.get_or_create(&config).await?;
            metadata::list_databases(&client).await
        })
        .await
    }

    async fn list_collections(
        &self,
        config: &ConnectionConfig,
        db: &str,
    ) -> Result<Vec<MongoCollection>> {
        let config = config.clone();
        let db = db.to_string();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let client = pools.get_or_create(&config).await?;
            metadata::list_collections(&client, &db).await
        })
        .await
    }

    async fn list_indexes(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
    ) -> Result<Vec<MongoIndex>> {
        let config = config.clone();
        let db = db.to_string();
        let coll = coll.to_string();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let client = pools.get_or_create(&config).await?;
            metadata::list_indexes(&client, &db, &coll).await
        })
        .await
    }

    async fn collection_stats(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
    ) -> Result<MongoCollectionStats> {
        let config = config.clone();
        let db = db.to_string();
        let coll = coll.to_string();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let client = pools.get_or_create(&config).await?;
            metadata::collection_stats(&client, &db, &coll).await
        })
        .await
    }

    async fn find(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        spec: &MongoQuerySpec,
    ) -> Result<MongoQueryResult> {
        let config = config.clone();
        let db = db.to_string();
        let coll = coll.to_string();
        let spec = spec.clone();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let client = pools.get_or_create(&config).await?;
            query::find(&client, &db, &coll, &spec).await
        })
        .await
    }

    async fn count(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        filter: &MongoDocument,
    ) -> Result<u64> {
        let config = config.clone();
        let db = db.to_string();
        let coll = coll.to_string();
        let filter = filter.clone();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let client = pools.get_or_create(&config).await?;
            query::count(&client, &db, &coll, filter).await
        })
        .await
    }

    async fn aggregate(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        pipeline: Vec<MongoDocument>,
    ) -> Result<MongoQueryResult> {
        let config = config.clone();
        let db = db.to_string();
        let coll = coll.to_string();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let client = pools.get_or_create(&config).await?;
            query::aggregate(&client, &db, &coll, pipeline).await
        })
        .await
    }

    async fn insert_one(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        document: MongoDocument,
    ) -> Result<String> {
        let config = config.clone();
        let db = db.to_string();
        let coll = coll.to_string();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let client = pools.get_or_create(&config).await?;
            query::insert_one(&client, &db, &coll, document).await
        })
        .await
    }

    async fn update_one(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        filter: &MongoDocument,
        update: &MongoDocument,
    ) -> Result<MongoQueryResult> {
        let config = config.clone();
        let db = db.to_string();
        let coll = coll.to_string();
        let filter = filter.clone();
        let update = update.clone();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let client = pools.get_or_create(&config).await?;
            query::update_one(&client, &db, &coll, filter, update).await
        })
        .await
    }

    async fn delete_one(
        &self,
        config: &ConnectionConfig,
        db: &str,
        coll: &str,
        filter: &MongoDocument,
    ) -> Result<MongoQueryResult> {
        let config = config.clone();
        let db = db.to_string();
        let coll = coll.to_string();
        let filter = filter.clone();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let client = pools.get_or_create(&config).await?;
            query::delete_one(&client, &db, &coll, filter).await
        })
        .await
    }

    async fn run_command(
        &self,
        config: &ConnectionConfig,
        db: &str,
        command: MongoDocument,
    ) -> Result<MongoDocument> {
        let config = config.clone();
        let db = db.to_string();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let client = pools.get_or_create(&config).await?;
            query::run_command(&client, &db, command).await
        })
        .await
    }

    fn evict_pool(&self, id: &ConnectionId) {
        self.pools.evict(id);
    }
}
