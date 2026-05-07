//! 集成测试：连接真实 MySQL。缺 RAMAG_TEST_MYSQL_* 环境变量时跳过

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ramag_domain::entities::{ConnectionConfig, ConnectionId, DriverKind, Query};
use ramag_domain::traits::Driver;
use ramag_infra_mysql::MysqlDriver;

/// 缺任一字段就跳过测试
fn config_from_env() -> Option<ConnectionConfig> {
    let host = std::env::var("RAMAG_TEST_MYSQL_HOST").ok()?;
    let port: u16 = std::env::var("RAMAG_TEST_MYSQL_PORT").ok()?.parse().ok()?;
    let user = std::env::var("RAMAG_TEST_MYSQL_USER").ok()?;
    let password = std::env::var("RAMAG_TEST_MYSQL_PASSWORD").ok()?;
    let database = std::env::var("RAMAG_TEST_MYSQL_DB").ok();

    Some(ConnectionConfig {
        id: ConnectionId::new(),
        name: "integration-test".into(),
        driver: DriverKind::Mysql,
        host,
        port,
        username: user,
        password,
        database,
        remark: None,
        color: Default::default(),
    })
}

/// 缺环境变量时打印 skip 提示再 return
macro_rules! require_env {
    () => {{
        match config_from_env() {
            Some(c) => c,
            None => {
                eprintln!(
                    "[SKIP] integration test skipped: 设置 RAMAG_TEST_MYSQL_* 环境变量后运行"
                );
                return;
            }
        }
    }};
}

#[tokio::test(flavor = "multi_thread")]
async fn test_connection_works() {
    let config = require_env!();
    let driver = MysqlDriver::new();
    driver
        .test_connection(&config)
        .await
        .expect("test_connection 失败");
}

#[tokio::test(flavor = "multi_thread")]
async fn list_schemas_returns_data() {
    let config = require_env!();
    let driver = MysqlDriver::new();
    let schemas = driver
        .list_schemas(&config)
        .await
        .expect("list_schemas 失败");
    println!("schemas: {:#?}", schemas);
    assert!(!schemas.is_empty(), "至少应返回一个 schema");
}

#[tokio::test(flavor = "multi_thread")]
async fn list_tables_for_db() {
    let config = require_env!();
    let driver = MysqlDriver::new();
    let schema = config
        .database
        .clone()
        .unwrap_or_else(|| "midas_storage".into());
    let tables = driver
        .list_tables(&config, &schema)
        .await
        .expect("list_tables 失败");
    println!("tables in {}: {:#?}", schema, tables);
    // 不强制有表，只验证调用成功
}

#[tokio::test(flavor = "multi_thread")]
async fn list_columns_for_first_table() {
    let config = require_env!();
    let driver = MysqlDriver::new();
    let schema = config
        .database
        .clone()
        .unwrap_or_else(|| "midas_storage".into());

    let tables = driver
        .list_tables(&config, &schema)
        .await
        .expect("list_tables 失败");

    if let Some(first_table) = tables.first() {
        let columns = driver
            .list_columns(&config, &schema, &first_table.name)
            .await
            .expect("list_columns 失败");
        println!("columns of {}.{}: {:#?}", schema, first_table.name, columns);
        assert!(!columns.is_empty(), "表应至少有一列");
    } else {
        eprintln!("[INFO] 库 {} 没有表，跳过列检查", schema);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn execute_select_one() {
    let config = require_env!();
    let driver = MysqlDriver::new();

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
async fn execute_select_with_types() {
    let config = require_env!();
    let driver = MysqlDriver::new();

    let result = driver
        .execute(
            &config,
            &Query::new(
                "SELECT \
                    1 AS i, \
                    1.5 AS f, \
                    'text' AS t, \
                    NULL AS n, \
                    NOW() AS dt, \
                    JSON_OBJECT('k', 'v') AS j",
            ),
        )
        .await
        .expect("execute 失败");

    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.columns.len(), 6);
    println!("typed result: {:#?}", result.rows[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_sql_returns_error() {
    let config = require_env!();
    let driver = MysqlDriver::new();

    let err = driver
        .execute(&config, &Query::new("SELEC * FORM x"))
        .await
        .expect_err("应该报语法错误");

    println!("got expected error: {}", err);
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_password_returns_friendly_error() {
    let mut config = require_env!();
    config.password = "definitely-wrong-password".to_string();

    let driver = MysqlDriver::new();
    let err = driver
        .test_connection(&config)
        .await
        .expect_err("应该报认证错误");

    println!("got expected auth error: {}", err);
    let msg = format!("{err}");
    assert!(
        msg.contains("用户名或密码") || msg.contains("Access denied") || msg.contains("1045"),
        "错误消息应包含认证错误线索，实际：{msg}"
    );
}
