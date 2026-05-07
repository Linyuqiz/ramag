//! 按 ConnectionConfig 构造 PgPool。缓存在 sql-shared::PoolCache

use std::time::Duration;

use ramag_domain::entities::{ConnectionConfig, DriverKind};
use ramag_domain::error::{DomainError, Result};
use sqlx::ConnectOptions;
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use tracing::warn;

use crate::errors::map_postgres_error;

/// 服务端 `pg_stat_activity` 识别 ramag 连接用
const DEFAULT_APPLICATION_NAME: &str = "ramag";

/// PG 必须指定 database，空时返回 InvalidConfig
pub async fn build_pool(config: &ConnectionConfig) -> Result<PgPool> {
    if config.driver != DriverKind::Postgres {
        return Err(DomainError::InvalidConfig(format!(
            "PostgresDriver 不支持 {:?} 类型连接",
            config.driver
        )));
    }
    let database = config
        .database
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            DomainError::InvalidConfig("PostgreSQL 必须指定具体数据库（database 字段必填）".into())
        })?;

    let opts = PgConnectOptions::new()
        .host(&config.host)
        .port(config.port)
        .username(&config.username)
        .password(&config.password)
        .database(database)
        .application_name(DEFAULT_APPLICATION_NAME)
        // 有 SSL 用 SSL，无则降级
        .ssl_mode(PgSslMode::Prefer)
        .log_statements(tracing::log::LevelFilter::Debug)
        .log_slow_statements(tracing::log::LevelFilter::Warn, Duration::from_secs(1));

    PgPoolOptions::new()
        .max_connections(8)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Some(Duration::from_secs(60 * 5)))
        .test_before_acquire(true)
        .connect_with(opts)
        .await
        .map_err(|e| {
            warn!(error = %e, host = %config.host, "build postgres pool failed");
            map_postgres_error(&e)
        })
}
