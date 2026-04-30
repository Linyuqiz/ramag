//! MySQL 连接池构造
//!
//! 连接池缓存逻辑由 `ramag-infra-sql-shared::PoolCache<MySql>` 提供。
//! 本模块只负责按 [`ConnectionConfig`] 构造一个新的 [`MySqlPool`]。

use std::time::Duration;

use ramag_domain::entities::{ConnectionConfig, DriverKind};
use ramag_domain::error::{DomainError, Result};
use sqlx::ConnectOptions;
use sqlx::MySqlPool;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions, MySqlSslMode};
use tracing::warn;

use crate::errors::map_mysql_error;

/// 按配置构造 sqlx 连接池
pub async fn build_pool(config: &ConnectionConfig) -> Result<MySqlPool> {
    if config.driver != DriverKind::Mysql {
        return Err(DomainError::InvalidConfig(format!(
            "MysqlDriver 不支持 {:?} 类型连接",
            config.driver
        )));
    }

    let opts = MySqlConnectOptions::new()
        .host(&config.host)
        .port(config.port)
        .username(&config.username)
        .password(&config.password)
        // utf8mb4 支持 emoji 与所有中文，强制要求
        .charset("utf8mb4")
        // 默认 UTC 避免歧义
        .timezone(Some("+00:00".into()))
        // SSL 默认偏好（有则用、无则降级）；后续 Stage 9 提供 UI 选项
        .ssl_mode(MySqlSslMode::Preferred)
        .log_statements(tracing::log::LevelFilter::Debug)
        .log_slow_statements(tracing::log::LevelFilter::Warn, Duration::from_secs(1));

    let opts = if let Some(db) = config.database.as_ref().filter(|s| !s.is_empty()) {
        opts.database(db)
    } else {
        opts
    };

    MySqlPoolOptions::new()
        .max_connections(8)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(10))
        .idle_timeout(Some(Duration::from_secs(60 * 5)))
        .test_before_acquire(true)
        .connect_with(opts)
        .await
        .map_err(|e| {
            warn!(error = %e, host = %config.host, "build mysql pool failed");
            map_mysql_error(&e)
        })
}
