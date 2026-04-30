//! PostgreSQL 连接池构造
//!
//! 连接池缓存逻辑由 `ramag-infra-sql-shared::PoolCache<Postgres>` 提供。
//! 本模块按 [`ConnectionConfig`] 构造一个新的 [`PgPool`]，含 sslmode 4 档配置 +
//! application_name 默认值。

use std::time::Duration;

use ramag_domain::entities::{ConnectionConfig, DriverKind};
use ramag_domain::error::{DomainError, Result};
use sqlx::ConnectOptions;
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use tracing::warn;

use crate::errors::map_postgres_error;

/// 默认 application_name（在服务端 `pg_stat_activity` 里识别 ramag 连接）
const DEFAULT_APPLICATION_NAME: &str = "ramag";

/// 按配置构造 sqlx 连接池
///
/// PG 必须连接到具体 database，本函数对空 database 直接返回 InvalidConfig
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
        // 默认 prefer 模式：有 SSL 用 SSL，无则降级；后续 Stage 9 暴露 UI 可选
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

/// 解析 sslmode 字符串到 sqlx PgSslMode；无效值默认 Prefer
///
/// 4 档：disable / require / verify-ca / verify-full（与 PG 协议保持一致）。
/// 当前 ConnectionConfig 没有 sslmode 字段（v0.3 后续 Stage 9 加），
/// 本函数留作后续扩展时启用
#[allow(dead_code)]
pub fn parse_sslmode(s: &str) -> PgSslMode {
    match s.to_ascii_lowercase().as_str() {
        "disable" => PgSslMode::Disable,
        "allow" => PgSslMode::Allow,
        "prefer" => PgSslMode::Prefer,
        "require" => PgSslMode::Require,
        "verify-ca" => PgSslMode::VerifyCa,
        "verify-full" => PgSslMode::VerifyFull,
        _ => PgSslMode::Prefer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sslmode_known() {
        assert!(matches!(parse_sslmode("disable"), PgSslMode::Disable));
        assert!(matches!(parse_sslmode("REQUIRE"), PgSslMode::Require));
        assert!(matches!(parse_sslmode("verify-ca"), PgSslMode::VerifyCa));
        assert!(matches!(
            parse_sslmode("verify-full"),
            PgSslMode::VerifyFull
        ));
    }

    #[test]
    fn parse_sslmode_unknown_falls_back_to_prefer() {
        assert!(matches!(parse_sslmode("xxxx"), PgSslMode::Prefer));
        assert!(matches!(parse_sslmode(""), PgSslMode::Prefer));
    }
}
