// 集成测试代码大量使用 unwrap/expect/panic（断言失败即测试失败），是 Rust 测试的常态
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! 集成测试：连接真实 Redis 跑通完整流程
//!
//! # 运行方式
//!
//! ```bash
//! # 设置环境变量后运行（否则 skip）
//! export RAMAG_TEST_REDIS_HOST=127.0.0.1
//! export RAMAG_TEST_REDIS_PORT=6379
//! # 可选（无密码可不设）
//! export RAMAG_TEST_REDIS_PASSWORD=secret
//! # 可选（ACL 用户名；空 = 走老版 AUTH 仅密码）
//! export RAMAG_TEST_REDIS_USERNAME=default
//!
//! cargo test -p ramag-infra-redis --test integration -- --nocapture
//! ```
//!
//! 测试用 db 15（避免污染常用的 0 号库）。**测试结尾会 FLUSHDB 清场**。

use ramag_domain::entities::{ConnectionConfig, ConnectionId, DriverKind, RedisType, RedisValue};
use ramag_domain::traits::KvDriver;
use ramag_infra_redis::RedisDriver;

const TEST_DB: u8 = 15;

/// 从环境变量读取连接配置；缺关键字段就跳过测试
fn config_from_env() -> Option<ConnectionConfig> {
    let host = std::env::var("RAMAG_TEST_REDIS_HOST").ok()?;
    let port: u16 = std::env::var("RAMAG_TEST_REDIS_PORT").ok()?.parse().ok()?;
    let password = std::env::var("RAMAG_TEST_REDIS_PASSWORD").unwrap_or_default();
    let username = std::env::var("RAMAG_TEST_REDIS_USERNAME").unwrap_or_default();

    Some(ConnectionConfig {
        id: ConnectionId::new(),
        name: "redis-integration-test".into(),
        driver: DriverKind::Redis,
        host,
        port,
        username,
        password,
        database: None,
        remark: None,
        color: Default::default(),
    })
}

macro_rules! require_env {
    () => {{
        match config_from_env() {
            Some(c) => c,
            None => {
                eprintln!(
                    "[SKIP] integration test skipped: 设置 RAMAG_TEST_REDIS_HOST/PORT 环境变量后运行"
                );
                return;
            }
        }
    }};
}

async fn cleanup(driver: &RedisDriver, config: &ConnectionConfig) {
    let _ = driver
        .execute_command(config, TEST_DB, vec!["FLUSHDB".into()])
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_connection_works() {
    let config = require_env!();
    let driver = RedisDriver::new();
    driver
        .test_connection(&config)
        .await
        .expect("test_connection 失败");
}

#[tokio::test(flavor = "multi_thread")]
async fn server_version_returns_value() {
    let config = require_env!();
    let driver = RedisDriver::new();
    let v = driver
        .server_version(&config)
        .await
        .expect("server_version 失败");
    println!("redis_version: {v}");
    assert!(!v.is_empty());
    assert_ne!(v, "unknown");
}

#[tokio::test(flavor = "multi_thread")]
async fn db_size_and_dbsize_command_match() {
    let config = require_env!();
    let driver = RedisDriver::new();
    cleanup(&driver, &config).await;

    let n0 = driver.db_size(&config, TEST_DB).await.unwrap();
    assert_eq!(n0, 0, "FLUSHDB 后应为 0");

    driver
        .execute_command(
            &config,
            TEST_DB,
            vec!["SET".into(), "ping_key".into(), "ok".into()],
        )
        .await
        .unwrap();

    let n1 = driver.db_size(&config, TEST_DB).await.unwrap();
    assert_eq!(n1, 1);

    cleanup(&driver, &config).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn string_get_set_roundtrip() {
    let config = require_env!();
    let driver = RedisDriver::new();
    cleanup(&driver, &config).await;

    driver
        .execute_command(
            &config,
            TEST_DB,
            vec!["SET".into(), "greet".into(), "hello".into()],
        )
        .await
        .unwrap();

    let v = driver.get_value(&config, TEST_DB, "greet").await.unwrap();
    assert!(matches!(v, RedisValue::Text(s) if s == "hello"));

    let t = driver.key_type(&config, TEST_DB, "greet").await.unwrap();
    assert_eq!(t, RedisType::String);

    cleanup(&driver, &config).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hash_value_returns_pairs() {
    let config = require_env!();
    let driver = RedisDriver::new();
    cleanup(&driver, &config).await;

    driver
        .execute_command(
            &config,
            TEST_DB,
            vec![
                "HSET".into(),
                "user:1".into(),
                "name".into(),
                "alice".into(),
                "age".into(),
                "30".into(),
            ],
        )
        .await
        .unwrap();

    let v = driver.get_value(&config, TEST_DB, "user:1").await.unwrap();
    match v {
        RedisValue::Hash(pairs) => {
            assert_eq!(pairs.len(), 2);
            let names: Vec<_> = pairs.iter().map(|(k, _)| k.as_str()).collect();
            assert!(names.contains(&"name"));
            assert!(names.contains(&"age"));
        }
        other => panic!("期望 Hash，实得 {other:?}"),
    }

    cleanup(&driver, &config).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn list_value_preserves_order() {
    let config = require_env!();
    let driver = RedisDriver::new();
    cleanup(&driver, &config).await;

    driver
        .execute_command(
            &config,
            TEST_DB,
            vec![
                "RPUSH".into(),
                "l".into(),
                "a".into(),
                "b".into(),
                "c".into(),
            ],
        )
        .await
        .unwrap();

    let v = driver.get_value(&config, TEST_DB, "l").await.unwrap();
    match v {
        RedisValue::List(elems) => {
            assert_eq!(elems.len(), 3);
            assert!(matches!(&elems[0], RedisValue::Text(s) if s == "a"));
            assert!(matches!(&elems[2], RedisValue::Text(s) if s == "c"));
        }
        other => panic!("期望 List，实得 {other:?}"),
    }

    cleanup(&driver, &config).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn zset_value_with_scores() {
    let config = require_env!();
    let driver = RedisDriver::new();
    cleanup(&driver, &config).await;

    driver
        .execute_command(
            &config,
            TEST_DB,
            vec![
                "ZADD".into(),
                "scores".into(),
                "1.5".into(),
                "alice".into(),
                "2.5".into(),
                "bob".into(),
            ],
        )
        .await
        .unwrap();

    let v = driver.get_value(&config, TEST_DB, "scores").await.unwrap();
    match v {
        RedisValue::ZSet(pairs) => {
            assert_eq!(pairs.len(), 2);
            // 服务端按 score 升序
            assert!((pairs[0].1 - 1.5).abs() < 1e-9);
            assert!((pairs[1].1 - 2.5).abs() < 1e-9);
        }
        other => panic!("期望 ZSet，实得 {other:?}"),
    }

    cleanup(&driver, &config).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn scan_iterates_full_keyspace() {
    let config = require_env!();
    let driver = RedisDriver::new();
    cleanup(&driver, &config).await;

    // 写 30 个 key
    for i in 0..30 {
        driver
            .execute_command(
                &config,
                TEST_DB,
                vec!["SET".into(), format!("scan:{i}"), "v".into()],
            )
            .await
            .unwrap();
    }

    let mut cursor = 0u64;
    let mut total = 0;
    loop {
        let r = driver
            .scan(&config, TEST_DB, cursor, Some("scan:*"), None, 10)
            .await
            .unwrap();
        total += r.keys.len();
        cursor = r.cursor;
        if cursor == 0 {
            break;
        }
    }
    assert_eq!(total, 30);

    cleanup(&driver, &config).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ttl_set_and_persist() {
    let config = require_env!();
    let driver = RedisDriver::new();
    cleanup(&driver, &config).await;

    driver
        .execute_command(
            &config,
            TEST_DB,
            vec!["SET".into(), "ttl_key".into(), "v".into()],
        )
        .await
        .unwrap();

    // 初始无 TTL
    let ttl = driver.key_ttl(&config, TEST_DB, "ttl_key").await.unwrap();
    assert_eq!(ttl, -1, "无 TTL 应返回 -1");

    // 设置 600s TTL
    let ok = driver
        .set_ttl(&config, TEST_DB, "ttl_key", Some(600))
        .await
        .unwrap();
    assert!(ok);

    let ttl_ms = driver.key_ttl(&config, TEST_DB, "ttl_key").await.unwrap();
    assert!(ttl_ms > 0 && ttl_ms <= 600_000);

    // 取消 TTL
    let ok = driver
        .set_ttl(&config, TEST_DB, "ttl_key", None)
        .await
        .unwrap();
    assert!(ok);

    let ttl = driver.key_ttl(&config, TEST_DB, "ttl_key").await.unwrap();
    assert_eq!(ttl, -1);

    cleanup(&driver, &config).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_returns_correct_bool() {
    let config = require_env!();
    let driver = RedisDriver::new();
    cleanup(&driver, &config).await;

    driver
        .execute_command(
            &config,
            TEST_DB,
            vec!["SET".into(), "del_target".into(), "v".into()],
        )
        .await
        .unwrap();

    let r = driver
        .delete_key(&config, TEST_DB, "del_target")
        .await
        .unwrap();
    assert!(r, "存在的 key 删除应返回 true");

    let r = driver
        .delete_key(&config, TEST_DB, "del_target")
        .await
        .unwrap();
    assert!(!r, "不存在的 key 删除应返回 false");

    cleanup(&driver, &config).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_key_returns_nil() {
    let config = require_env!();
    let driver = RedisDriver::new();
    cleanup(&driver, &config).await;

    let v = driver
        .get_value(&config, TEST_DB, "definitely_missing")
        .await
        .unwrap();
    assert!(matches!(v, RedisValue::Nil));

    let t = driver
        .key_type(&config, TEST_DB, "definitely_missing")
        .await
        .unwrap();
    assert_eq!(t, RedisType::None);
}
