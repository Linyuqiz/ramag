//! 集成测试：连接真实 PostgreSQL 跑通完整流程
//!
//! # 运行方式
//!
//! ```bash
//! # 设置环境变量后运行（任一字段缺失就 skip）
//! export RAMAG_TEST_PG_HOST=127.0.0.1
//! export RAMAG_TEST_PG_PORT=5432
//! export RAMAG_TEST_PG_USER=postgres
//! export RAMAG_TEST_PG_PASSWORD='your-password'
//! export RAMAG_TEST_PG_DB=postgres   # PG 必须连具体 db
//!
//! cargo test -p ramag-infra-postgres --test integration -- --nocapture
//! ```

// 测试代码局部豁免 unwrap/expect/panic：失败即用例失败，不需要 graceful 处理
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ramag_domain::entities::{ConnectionConfig, ConnectionId, DriverKind, Query};
use ramag_domain::traits::Driver;
use ramag_infra_postgres::PostgresDriver;

/// 从环境变量读取连接配置；缺任一字段就跳过测试
///
/// PG 必须连接具体 database，所以 `RAMAG_TEST_PG_DB` 也是必填
fn config_from_env() -> Option<ConnectionConfig> {
    let host = std::env::var("RAMAG_TEST_PG_HOST").ok()?;
    let port: u16 = std::env::var("RAMAG_TEST_PG_PORT").ok()?.parse().ok()?;
    let user = std::env::var("RAMAG_TEST_PG_USER").ok()?;
    let password = std::env::var("RAMAG_TEST_PG_PASSWORD").ok()?;
    let database = std::env::var("RAMAG_TEST_PG_DB").ok()?;

    Some(ConnectionConfig {
        id: ConnectionId::new(),
        name: "integration-test".into(),
        driver: DriverKind::Postgres,
        host,
        port,
        username: user,
        password,
        database: Some(database),
        remark: None,
        color: Default::default(),
    })
}

/// 跳过日志：方便定位 skip 原因
macro_rules! require_env {
    () => {{
        match config_from_env() {
            Some(c) => c,
            None => {
                eprintln!(
                    "[SKIP] integration test skipped: 设置 RAMAG_TEST_PG_* 环境变量后运行"
                );
                return;
            }
        }
    }};
}

#[tokio::test(flavor = "multi_thread")]
async fn test_connection_works() {
    let config = require_env!();
    let driver = PostgresDriver::new();
    driver
        .test_connection(&config)
        .await
        .expect("test_connection 失败");
}

#[tokio::test(flavor = "multi_thread")]
async fn server_version_returns_value() {
    let config = require_env!();
    let driver = PostgresDriver::new();
    let v = driver
        .server_version(&config)
        .await
        .expect("server_version 失败");
    println!("postgres version: {v}");
    assert!(!v.is_empty(), "版本字符串应非空");
}

#[tokio::test(flavor = "multi_thread")]
async fn list_schemas_returns_data() {
    let config = require_env!();
    let driver = PostgresDriver::new();
    let schemas = driver
        .list_schemas(&config)
        .await
        .expect("list_schemas 失败");
    println!("schemas: {:#?}", schemas);
    // 至少应有 public（PG 默认 schema）；filter 排除了 pg_*/information_schema
    assert!(
        schemas.iter().any(|s| s.name == "public"),
        "应包含 public schema"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn list_tables_for_public() {
    let config = require_env!();
    let driver = PostgresDriver::new();
    let tables = driver
        .list_tables(&config, "public")
        .await
        .expect("list_tables 失败");
    println!("tables in public: {:#?}", tables);
    // 不强制有表（库可能为空），只验证调用成功
}

#[tokio::test(flavor = "multi_thread")]
async fn execute_select_one() {
    let config = require_env!();
    let driver = PostgresDriver::new();

    let result = driver
        .execute(&config, &Query::new("SELECT 1 AS one, 'hello' AS greet"))
        .await
        .expect("execute 失败");

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.columns.len(), 2);
    assert_eq!(result.affected_rows, 0);
    println!(
        "result: cols={:?}, rows={:?}, elapsed={}ms",
        result.columns, result.rows, result.elapsed_ms
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn execute_select_with_pg_types() {
    let config = require_env!();
    let driver = PostgresDriver::new();

    // 覆盖 PG 关键类型映射：bool / int / float / numeric(精度) / text / timestamp / jsonb / uuid
    let result = driver
        .execute(
            &config,
            &Query::new(
                "SELECT \
                    true AS b, \
                    42::int4 AS i, \
                    1.5::float8 AS f, \
                    1234567890123456789012.34::numeric AS n, \
                    'text'::text AS t, \
                    NULL AS null_col, \
                    NOW() AS ts, \
                    '{\"k\": \"v\"}'::jsonb AS j, \
                    '11111111-1111-1111-1111-111111111111'::uuid AS u",
            ),
        )
        .await
        .expect("execute 失败");

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.columns.len(), 9);
    println!("typed result: {:#?}", result.rows[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_sql_returns_error() {
    let config = require_env!();
    let driver = PostgresDriver::new();

    let err = driver
        .execute(&config, &Query::new("SELEC * FORM x"))
        .await
        .expect_err("应该报语法错误");

    println!("got expected error: {}", err);
    let msg = format!("{err}");
    // PG 语法错误对应 SQLSTATE 42601 → DomainError::QueryFailed("SQL 语法错误...")
    assert!(
        msg.contains("语法") || msg.contains("syntax"),
        "错误消息应包含语法错误线索：{msg}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_password_returns_friendly_error() {
    let mut config = require_env!();
    config.password = "definitely-wrong-password".to_string();

    let driver = PostgresDriver::new();
    let err = driver
        .test_connection(&config)
        .await
        .expect_err("应该报认证错误");

    println!("got expected auth error: {}", err);
    let msg = format!("{err}");
    // PG 认证错误 SQLSTATE 28P01 → "用户名或密码错误"
    assert!(
        msg.contains("用户名或密码") || msg.contains("password") || msg.contains("authentication"),
        "错误消息应包含认证错误线索：{msg}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_result_set_keeps_columns() {
    // P0 修复验证：SELECT WHERE 1=0 返回空 rows 但有列头
    let config = require_env!();
    let driver = PostgresDriver::new();

    let result = driver
        .execute(&config, &Query::new("SELECT 1 AS a, 'x' AS b WHERE 1 = 0"))
        .await
        .expect("execute 失败");

    assert!(result.rows.is_empty(), "WHERE 1=0 应返回空 rows");
    // 关键：列头不应丢失（extract_columns_fallback 走 describe）
    assert_eq!(result.columns.len(), 2, "空结果集仍应有列定义");
    println!("empty result columns: {:?}", result.columns);
}

#[tokio::test(flavor = "multi_thread")]
async fn dollar_quoted_function_body_treated_as_one_statement() {
    // PG dollar-quoted 多语句切分验证：函数体内 ; 不分割
    let config = require_env!();
    let driver = PostgresDriver::new();

    // 创建一个临时函数 + 紧跟 SELECT 2，多语句一次执行
    // 函数体内有 ; 但应整体作为一条语句
    let sql = "DO $$ BEGIN PERFORM 1; PERFORM 2; END; $$; SELECT 99 AS final_value";
    let result = driver
        .execute(&config, &Query::new(sql))
        .await
        .expect("dollar-quoted 多语句执行失败");

    // 最后一条是 SELECT 99，结果应是单行 (final_value=99)
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.columns.len(), 1);
    assert_eq!(result.columns[0], "final_value");
    println!("dollar-quoted result: {:?}", result.rows[0]);
}
