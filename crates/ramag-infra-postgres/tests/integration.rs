//! 集成测试：连接真实 PostgreSQL。缺 RAMAG_TEST_PG_* 环境变量时跳过。PG 必须指定 db

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ramag_domain::entities::{ConnectionConfig, ConnectionId, DriverKind, Query};
use ramag_domain::traits::Driver;
use ramag_infra_postgres::PostgresDriver;

/// 缺任一字段就跳过测试。PG 必须指定 database，`RAMAG_TEST_PG_DB` 必填
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
        auth_source: None,
        remark: None,
        production: false,
    })
}

/// 缺环境变量时打印 skip 提示再 return
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
    // PG 默认有 public
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
    // 不强制有表，只验证调用成功
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
    // 42601 → "SQL 语法错误"
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
    // 28P01 → "用户名或密码错误"
    assert!(
        msg.contains("用户名或密码") || msg.contains("password") || msg.contains("authentication"),
        "错误消息应包含认证错误线索：{msg}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_result_set_keeps_columns() {
    let config = require_env!();
    let driver = PostgresDriver::new();

    let result = driver
        .execute(&config, &Query::new("SELECT 1 AS a, 'x' AS b WHERE 1 = 0"))
        .await
        .expect("execute 失败");

    assert!(result.rows.is_empty(), "WHERE 1=0 应返回空 rows");
    // extract_columns_fallback 经 describe 拿列头
    assert_eq!(result.columns.len(), 2, "空结果集仍应有列定义");
    println!("empty result columns: {:?}", result.columns);
}

#[tokio::test(flavor = "multi_thread")]
async fn dollar_quoted_function_body_treated_as_one_statement() {
    let config = require_env!();
    let driver = PostgresDriver::new();

    // dollar-quoted 函数体内的 ; 不应被切分
    let sql = "DO $$ BEGIN PERFORM 1; PERFORM 2; END; $$; SELECT 99 AS final_value";
    let result = driver
        .execute(&config, &Query::new(sql))
        .await
        .expect("dollar-quoted 多语句执行失败");

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.columns.len(), 1);
    assert_eq!(result.columns[0], "final_value");
    println!("dollar-quoted result: {:?}", result.rows[0]);
}
