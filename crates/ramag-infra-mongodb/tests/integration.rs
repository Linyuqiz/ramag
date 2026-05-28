#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! 集成测试：连真实 MongoDB（默认 skip，缺环境变量直接 return，不让 `make test` 失败）。
//!
//! 必填环境变量（缺任一即 skip）：
//!   RAMAG_TEST_MONGO_HOST  (例 127.0.0.1)
//!   RAMAG_TEST_MONGO_PORT  (例 27017)
//!   RAMAG_TEST_MONGO_DB    (例 ramag_test)
//! 可选：
//!   RAMAG_TEST_MONGO_USER
//!   RAMAG_TEST_MONGO_PASSWORD
//!
//! 跑法：
//!   cargo test -p ramag-infra-mongodb --test integration -- --nocapture

use ramag_domain::entities::ConnectionConfig;
use ramag_domain::traits::DocDriver;
use ramag_infra_mongodb::MongoDriver;
use serde_json::json;

fn build_config_from_env() -> Option<ConnectionConfig> {
    let host = std::env::var("RAMAG_TEST_MONGO_HOST").ok()?;
    let port: u16 = std::env::var("RAMAG_TEST_MONGO_PORT").ok()?.parse().ok()?;
    let db = std::env::var("RAMAG_TEST_MONGO_DB").ok()?;

    let mut cfg = ConnectionConfig::new_mongodb("integration", host, port);
    if let Ok(user) = std::env::var("RAMAG_TEST_MONGO_USER") {
        cfg.username = user;
    }
    if let Ok(pwd) = std::env::var("RAMAG_TEST_MONGO_PASSWORD") {
        cfg.password = pwd;
    }
    cfg.database = Some(db);
    Some(cfg)
}

#[tokio::test]
async fn test_connection_and_list() {
    let Some(cfg) = build_config_from_env() else {
        eprintln!("skip: env not set");
        return;
    };
    let driver = MongoDriver::new();
    driver.test_connection(&cfg).await.expect("ping failed");

    let ver = driver.server_version(&cfg).await.expect("buildInfo failed");
    eprintln!("server_version={ver}");
    assert!(!ver.is_empty());

    let dbs = driver
        .list_databases(&cfg)
        .await
        .expect("list_databases failed");
    eprintln!("databases={}", dbs.len());
    assert!(!dbs.is_empty());
}

#[tokio::test]
async fn test_crud_roundtrip() {
    let Some(cfg) = build_config_from_env() else {
        eprintln!("skip: env not set");
        return;
    };
    let Some(db) = cfg.database.clone() else {
        eprintln!("skip: db env not set");
        return;
    };
    let driver = MongoDriver::new();
    let coll = "ramag_test_integration";

    // 插入
    let id = driver
        .insert_one(&cfg, &db, coll, json!({"name": "alice", "age": 30}))
        .await
        .expect("insert failed");
    eprintln!("inserted id={id}");
    assert!(!id.is_empty());

    // 统计
    let n = driver
        .count(&cfg, &db, coll, &json!({"name": "alice"}))
        .await
        .expect("count failed");
    assert!(n >= 1);

    // 查找
    use ramag_domain::entities::MongoQuerySpec;
    let spec = MongoQuerySpec {
        filter: json!({"name": "alice"}),
        limit: Some(10),
        ..Default::default()
    };
    let result = driver
        .find(&cfg, &db, coll, &spec)
        .await
        .expect("find failed");
    assert!(!result.documents.is_empty());

    // 清理
    let _ = driver
        .delete_one(&cfg, &db, coll, &json!({"name": "alice"}))
        .await;
}

/// 深度查询：对 ramag_demo 库的 users / products / orders 跑完整方法集。
/// 需先用 docker mongosh 灌入种子数据；不存在时 skip
#[tokio::test]
async fn test_demo_data_full_queries() {
    let Some(cfg) = build_config_from_env() else {
        eprintln!("skip: env not set");
        return;
    };
    let Some(db) = cfg.database.clone() else {
        return;
    };
    if db != "ramag_demo" {
        eprintln!("skip: 仅在 RAMAG_TEST_MONGO_DB=ramag_demo 时跑");
        return;
    }
    let driver = MongoDriver::new();

    // 1) list_collections
    let colls = driver
        .list_collections(&cfg, &db)
        .await
        .expect("list_collections failed");
    let names: Vec<&str> = colls.iter().map(|c| c.name.as_str()).collect();
    eprintln!("collections: {}", names.join(", "));
    assert!(names.contains(&"users"));
    assert!(names.contains(&"products"));
    assert!(names.contains(&"orders"));

    // 2) list_indexes(users) — 含 _id_ / idx_age / idx_email_uniq
    let idxs = driver
        .list_indexes(&cfg, &db, "users")
        .await
        .expect("list_indexes failed");
    let idx_names: Vec<&str> = idxs.iter().map(|i| i.name.as_str()).collect();
    eprintln!("users indexes: {}", idx_names.join(", "));
    assert!(idx_names.contains(&"_id_"));
    assert!(idx_names.contains(&"idx_age"));
    assert!(idx_names.contains(&"idx_email_uniq"));
    assert!(idxs.iter().any(|i| i.name == "idx_email_uniq" && i.unique));

    // 3) collection_stats
    let stats = driver
        .collection_stats(&cfg, &db, "users")
        .await
        .expect("stats failed");
    eprintln!(
        "users stats: count={} size={} indexes={}",
        stats.count, stats.size_bytes, stats.index_count
    );
    assert!(stats.count >= 10);
    assert!(stats.index_count >= 3);

    // 4) count 全量 + 条件
    let n_all = driver
        .count(&cfg, &db, "users", &json!({}))
        .await
        .expect("count all failed");
    let n_admin = driver
        .count(&cfg, &db, "users", &json!({"role": "admin"}))
        .await
        .expect("count admin failed");
    eprintln!("users count: all={n_all} admin={n_admin}");
    assert!(n_all >= 10);
    assert!(n_admin >= 2);

    // 5) find: age>=30 倒序 limit=5
    use ramag_domain::entities::MongoQuerySpec;
    let spec = MongoQuerySpec {
        filter: json!({"age": {"$gte": 30}}),
        sort: Some(json!({"age": -1})),
        limit: Some(5),
        ..Default::default()
    };
    let r = driver
        .find(&cfg, &db, "users", &spec)
        .await
        .expect("find failed");
    eprintln!("find users age>=30 desc limit5: {} docs", r.documents.len());
    for d in &r.documents {
        eprintln!("  {}", d);
    }
    assert!(!r.documents.is_empty());
    assert!(r.documents.len() <= 5);

    // 6) aggregate: 按 role 分组
    let pipeline = vec![
        json!({"$group": {"_id": "$role", "count": {"$sum": 1}}}),
        json!({"$sort": {"count": -1}}),
    ];
    let r = driver
        .aggregate(&cfg, &db, "users", pipeline)
        .await
        .expect("aggregate failed");
    eprintln!("aggregate users by role: {} groups", r.documents.len());
    for d in &r.documents {
        eprintln!("  {}", d);
    }
    assert!(r.documents.len() >= 2);

    // 7) products: Decimal128 字段 find，确认 Extended JSON 编码 $numberDecimal
    let spec = MongoQuerySpec {
        filter: json!({"category": "electronics"}),
        limit: Some(3),
        ..Default::default()
    };
    let r = driver
        .find(&cfg, &db, "products", &spec)
        .await
        .expect("find products failed");
    eprintln!("electronics products: {} docs", r.documents.len());
    for d in &r.documents {
        eprintln!("  {}", d);
    }
    let any_decimal = r
        .documents
        .iter()
        .any(|d| d.to_string().contains("$numberDecimal"));
    assert!(any_decimal, "Decimal128 字段应编码为 $numberDecimal");

    // 8) run_command: dbStats 兜底通用命令
    let stats = driver
        .run_command(&cfg, &db, json!({"dbStats": 1}))
        .await
        .expect("run_command dbStats failed");
    eprintln!("dbStats: {stats}");
    assert!(stats.get("collections").is_some());
}
