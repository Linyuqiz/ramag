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

/// 复现 UI 单元格编辑路径：insert → find 取回 Extended JSON 形式的 _id → 用它构造
/// filter={_id} + update={$set} → update_one → 回查确认。验证 ObjectId _id 往返后能否匹配更新。
#[tokio::test]
async fn test_update_one_reproduce() {
    let Some(cfg) = build_config_from_env() else {
        eprintln!("skip: env not set");
        return;
    };
    let Some(db) = cfg.database.clone() else {
        return;
    };
    let driver = MongoDriver::new();
    let coll = "ramag_update_probe";

    // 清场（避免上次残留）
    let _ = driver
        .delete_one(&cfg, &db, coll, &json!({"name": "probe"}))
        .await;

    // 1) 插入文档（_id 由 mongo 生成 ObjectId）
    let inserted = driver
        .insert_one(&cfg, &db, coll, json!({"name": "probe", "age": 30}))
        .await
        .expect("insert failed");
    eprintln!("inserted _id(raw) = {inserted}");

    // 2) find 取回结果集，模拟 UI 拿到的文档（_id 为 Extended JSON 形式）
    use ramag_domain::entities::MongoQuerySpec;
    let spec = MongoQuerySpec {
        filter: json!({"name": "probe"}),
        limit: Some(1),
        ..Default::default()
    };
    let r = driver
        .find(&cfg, &db, coll, &spec)
        .await
        .expect("find failed");
    let doc = r.documents.first().expect("no doc found");
    let id = doc.get("_id").cloned().expect("doc has no _id");
    eprintln!("find returned _id(extjson) = {id}");

    // 3) 模拟单元格编辑：filter={_id}, update={$set:{age:31}}
    let filter = json!({ "_id": id });
    let update = json!({ "$set": { "age": 31 } });
    let res = driver
        .update_one(&cfg, &db, coll, &filter, &update)
        .await
        .expect("update_one failed");
    eprintln!("update_one affected(modified_count) = {}", res.affected);

    // 4) 回查确认 age 真的变成 31（绕过 affected 直接看数据）
    let r2 = driver
        .find(&cfg, &db, coll, &spec)
        .await
        .expect("re-find failed");
    let after = r2.documents.first().expect("doc gone after update");
    eprintln!("after update doc = {after}");
    assert_eq!(
        after.get("age").cloned(),
        Some(json!(31)),
        "age 应被更新为 31（若失败=filter 没匹配上 → 更新不生效）"
    );

    // 5) 改成相同值：affected 取 matched_count，应为 1（问题 A 回归：改相同值不再误判「未匹配」）
    let same = driver
        .update_one(&cfg, &db, coll, &filter, &json!({ "$set": { "age": 31 } }))
        .await
        .expect("update same failed");
    eprintln!(
        "update same-value affected(matched_count) = {}",
        same.affected
    );
    assert_eq!(
        same.affected, 1,
        "改相同值应仍 matched=1（affected 取 matched_count），否则 UI 会误报「未匹配」"
    );

    // 清理
    let _ = driver
        .delete_one(&cfg, &db, coll, &json!({"name": "probe"}))
        .await;
}

/// 探测某种 _id 类型能否走通 UI 更新路径：插入带该 _id 的文档 → find 取回 Extended JSON
/// 形式的 _id → 用它构造 filter={_id} 做 update_one → 回查 marker 是否被改。
/// 返回 true=filter 匹配成功（更新生效），false=matched 0（更新不了）。
async fn probe_id_roundtrip(
    driver: &MongoDriver,
    cfg: &ConnectionConfig,
    db: &str,
    coll: &str,
    doc: serde_json::Value,
    probe_tag: &str,
) -> bool {
    use ramag_domain::entities::MongoQuerySpec;
    driver
        .insert_one(cfg, db, coll, doc)
        .await
        .expect("insert failed");
    let spec = MongoQuerySpec {
        filter: json!({ "probe": probe_tag }),
        limit: Some(1),
        ..Default::default()
    };
    let r = driver
        .find(cfg, db, coll, &spec)
        .await
        .expect("find failed");
    let found = r
        .documents
        .first()
        .expect("inserted doc not found by probe");
    let id = found.get("_id").cloned().expect("no _id");
    eprintln!("[{probe_tag}] find 取回 _id = {id}");
    // 模拟 UI 单元格编辑：filter={_id}, update={$set:{marker:"updated"}}
    let filter = json!({ "_id": id });
    let update = json!({ "$set": { "marker": "updated" } });
    driver
        .update_one(cfg, db, coll, &filter, &update)
        .await
        .expect("update failed");
    // 用 probe 字段（不靠 _id）回查 marker 是否真被改
    let r2 = driver
        .find(cfg, db, coll, &spec)
        .await
        .expect("re-find failed");
    let marker = r2.documents.first().and_then(|d| d.get("marker")).cloned();
    let ok = marker == Some(json!("updated"));
    eprintln!("[{probe_tag}] 匹配成功={ok}（marker={marker:?}）");
    ok
}

/// _id 类型往返矩阵：逐类型走 UI 更新路径，列出哪些类型「更新不了」（matched 0）。
#[tokio::test]
async fn test_id_type_roundtrip_matrix() {
    let Some(cfg) = build_config_from_env() else {
        eprintln!("skip: env not set");
        return;
    };
    let Some(db) = cfg.database.clone() else {
        return;
    };
    let driver = MongoDriver::new();
    let coll = "ramag_idtype_probe";
    let _ = driver.run_command(&cfg, &db, json!({"drop": coll})).await;

    // 各类型文档：带 probe（稳定定位，不依赖 _id）+ marker（被更新的目标字段）
    let cases: Vec<(serde_json::Value, &str)> = vec![
        // 不指定 _id → mongo 自动 ObjectId（基线，应成功）
        (json!({"probe": "objectid", "marker": "orig"}), "objectid"),
        (
            json!({"_id": "str-id-1", "probe": "string", "marker": "orig"}),
            "string",
        ),
        (
            json!({"_id": 42, "probe": "int32", "marker": "orig"}),
            "int32",
        ),
        (
            json!({"_id": {"$numberLong": "1234567890123456789"}, "probe": "int64", "marker": "orig"}),
            "int64",
        ),
        (
            json!({"_id": {"region": "us", "seq": 7}, "probe": "compound", "marker": "orig"}),
            "compound",
        ),
        (
            json!({"_id": {"$numberDecimal": "100.50"}, "probe": "decimal", "marker": "orig"}),
            "decimal",
        ),
        (
            json!({"_id": {"$date": "2024-01-01T00:00:00Z"}, "probe": "date", "marker": "orig"}),
            "date",
        ),
    ];

    let mut failed = Vec::new();
    for (doc, tag) in cases {
        if !probe_id_roundtrip(&driver, &cfg, &db, coll, doc, tag).await {
            failed.push(tag);
        }
    }
    let _ = driver.run_command(&cfg, &db, json!({"drop": coll})).await;
    eprintln!("=== _id 往返「更新不了」的类型: {failed:?} ===");
    assert!(
        failed.is_empty(),
        "这些 _id 类型往返后 filter 匹配不上（更新不了）: {failed:?}"
    );
}
