//! 查询与写操作：find / count / aggregate / insert_one / update_one / delete_one / run_command / ping。
//! 全部走最小 API；options 通过 builder 链式装载

use std::time::Instant;

use bson::{Bson, Document, doc};
use futures::TryStreamExt;
use mongodb::Client;
use ramag_domain::entities::{MongoDocument, MongoQueryResult, MongoQuerySpec};
use ramag_domain::error::Result;
use serde_json::Value;

use crate::errors::map_mongo_error;
use crate::types::{document_to_json, json_to_document};

/// `ping` 命令，仅用于 test_connection
pub async fn ping(client: &Client) -> Result<()> {
    client
        .database("admin")
        .run_command(doc! {"ping": 1})
        .await
        .map_err(map_mongo_error)?;
    Ok(())
}

/// `buildInfo.version`
pub async fn server_version(client: &Client) -> Result<String> {
    let r: Document = client
        .database("admin")
        .run_command(doc! {"buildInfo": 1})
        .await
        .map_err(map_mongo_error)?;
    Ok(r.get_str("version")
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "unknown".to_string()))
}

pub async fn find(
    client: &Client,
    db: &str,
    coll: &str,
    spec: &MongoQuerySpec,
) -> Result<MongoQueryResult> {
    let start = Instant::now();

    let filter_doc = if spec.filter.is_null() {
        Document::new()
    } else {
        json_to_document(spec.filter.clone())?
    };

    let collection = client.database(db).collection::<Document>(coll);
    let mut find_action = collection.find(filter_doc);

    if let Some(skip) = spec.skip {
        find_action = find_action.skip(skip);
    }
    if let Some(limit) = spec.limit {
        find_action = find_action.limit(limit);
    }
    if let Some(sort) = &spec.sort
        && let Ok(doc) = json_to_document(sort.clone())
    {
        find_action = find_action.sort(doc);
    }
    if let Some(proj) = &spec.projection
        && let Ok(doc) = json_to_document(proj.clone())
    {
        find_action = find_action.projection(doc);
    }

    let mut cursor = find_action.await.map_err(map_mongo_error)?;
    let mut docs: Vec<MongoDocument> = Vec::new();
    while let Some(doc) = cursor.try_next().await.map_err(map_mongo_error)? {
        docs.push(document_to_json(doc));
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;
    Ok(MongoQueryResult::read(docs, elapsed_ms))
}

pub async fn count(client: &Client, db: &str, coll: &str, filter: MongoDocument) -> Result<u64> {
    let filter_doc = if filter.is_null() {
        Document::new()
    } else {
        json_to_document(filter)?
    };
    let collection = client.database(db).collection::<Document>(coll);
    let n = collection
        .count_documents(filter_doc)
        .await
        .map_err(map_mongo_error)?;
    Ok(n)
}

pub async fn aggregate(
    client: &Client,
    db: &str,
    coll: &str,
    pipeline: Vec<MongoDocument>,
) -> Result<MongoQueryResult> {
    let start = Instant::now();
    let mut docs_pipeline: Vec<Document> = Vec::with_capacity(pipeline.len());
    for stage in pipeline {
        docs_pipeline.push(json_to_document(stage)?);
    }
    let collection = client.database(db).collection::<Document>(coll);
    let mut cursor = collection
        .aggregate(docs_pipeline)
        .await
        .map_err(map_mongo_error)?;
    let mut out = Vec::new();
    while let Some(d) = cursor.try_next().await.map_err(map_mongo_error)? {
        out.push(document_to_json(d));
    }
    let elapsed_ms = start.elapsed().as_millis() as u64;
    Ok(MongoQueryResult::read(out, elapsed_ms))
}

pub async fn insert_one(
    client: &Client,
    db: &str,
    coll: &str,
    document: MongoDocument,
) -> Result<String> {
    let doc = json_to_document(document)?;
    let collection = client.database(db).collection::<Document>(coll);
    let r = collection.insert_one(doc).await.map_err(map_mongo_error)?;
    Ok(format_bson_id(&r.inserted_id))
}

pub async fn update_one(
    client: &Client,
    db: &str,
    coll: &str,
    filter: MongoDocument,
    update: MongoDocument,
) -> Result<MongoQueryResult> {
    let start = Instant::now();
    let filter_doc = json_to_document(filter)?;
    let update_doc = json_to_document(update)?;
    let collection = client.database(db).collection::<Document>(coll);
    let r = collection
        .update_one(filter_doc, update_doc)
        .await
        .map_err(map_mongo_error)?;
    tracing::info!(
        coll = coll,
        matched = r.matched_count,
        modified = r.modified_count,
        "mongo update_one done"
    );
    let elapsed_ms = start.elapsed().as_millis() as u64;
    // affected 取 matched_count（定位到的文档数）而非 modified_count：改成与原值相同时
    // modified=0，用 matched 才能正确反映「已定位」，避免上层把「值未变」误报成「未匹配」
    Ok(MongoQueryResult::write(
        r.matched_count,
        elapsed_ms,
        "updateOne",
    ))
}

pub async fn delete_one(
    client: &Client,
    db: &str,
    coll: &str,
    filter: MongoDocument,
) -> Result<MongoQueryResult> {
    let start = Instant::now();
    let filter_doc = json_to_document(filter)?;
    let collection = client.database(db).collection::<Document>(coll);
    let r = collection
        .delete_one(filter_doc)
        .await
        .map_err(map_mongo_error)?;
    let elapsed_ms = start.elapsed().as_millis() as u64;
    Ok(MongoQueryResult::write(
        r.deleted_count,
        elapsed_ms,
        "deleteOne",
    ))
}

/// 兜底任意命令。例：`dbStats` / `serverStatus` / `createIndexes`
pub async fn run_command(
    client: &Client,
    db: &str,
    command: MongoDocument,
) -> Result<MongoDocument> {
    let cmd_doc = json_to_document(command)?;
    let mut raw: Document = client
        .database(db)
        .run_command(cmd_doc)
        .await
        .map_err(map_mongo_error)?;
    // cursor 类命令（find / aggregate / listCollections…）首次只返回 firstBatch（默认 101 条），
    // 循环 getMore 补齐完整结果，否则数据不全（缺后续批次，含最新写入）
    if raw.get_document("cursor").is_ok() {
        drain_cursor(client, db, &mut raw).await?;
    }
    Ok(document_to_json(raw))
}

/// 把 cursor 响应补齐为完整结果：getMore 循环收集所有批次，写回 firstBatch 并把 id 归 0；
/// 下游按单批 firstBatch 解析即可拿到全部文档
async fn drain_cursor(client: &Client, db: &str, raw: &mut Document) -> Result<()> {
    // 上限保护：避免一次把超大集合全拉进内存（find 通常带 limit，远小于此）
    const MAX_DOCS: usize = 50_000;
    let (mut all, mut cursor_id, coll) = match raw.get_document("cursor") {
        Ok(c) => {
            let first = c.get_array("firstBatch").ok().cloned().unwrap_or_default();
            let id = c.get_i64("id").unwrap_or(0);
            // ns 形如 "db.collection"，取 collection 段做 getMore
            let coll = c
                .get_str("ns")
                .ok()
                .and_then(|ns| ns.split_once('.').map(|(_, coll)| coll.to_string()))
                .unwrap_or_default();
            (first, id, coll)
        }
        Err(_) => return Ok(()),
    };
    let database = client.database(db);
    while cursor_id != 0 && !coll.is_empty() && all.len() < MAX_DOCS {
        let more: Document = database
            .run_command(doc! {
                "getMore": cursor_id,
                "collection": coll.clone(),
                "batchSize": 4096i32,
            })
            .await
            .map_err(map_mongo_error)?;
        match more.get_document("cursor") {
            Ok(c) => {
                if let Ok(next) = c.get_array("nextBatch") {
                    all.extend(next.iter().cloned());
                }
                cursor_id = c.get_i64("id").unwrap_or(0);
            }
            Err(_) => break,
        }
    }
    if cursor_id != 0 {
        tracing::warn!(
            collected = all.len(),
            "mongo cursor truncated at safety cap"
        );
    }
    // 写回完整结果，id 归 0（下游按单批 firstBatch 解析即可拿到全部）
    if let Ok(c) = raw.get_document_mut("cursor") {
        c.insert("firstBatch", Bson::Array(all));
        c.insert("id", 0i64);
    }
    Ok(())
}

/// insertedId 是 Bson，常见 ObjectId / String / Int64；统一转可读字符串
fn format_bson_id(b: &Bson) -> String {
    let v: Value = b.clone().into_relaxed_extjson();
    match &v {
        Value::String(s) => s.clone(),
        Value::Object(map) => {
            if let Some(oid) = map.get("$oid").and_then(|x| x.as_str()) {
                return oid.to_string();
            }
            v.to_string()
        }
        _ => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::oid::ObjectId;

    #[test]
    fn format_objectid_extracts_hex() {
        let oid = ObjectId::new();
        let formatted = format_bson_id(&Bson::ObjectId(oid));
        assert_eq!(formatted.len(), 24);
        assert!(formatted.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn format_string_id_passthrough() {
        let v = Bson::String("custom-id".into());
        assert_eq!(format_bson_id(&v), "custom-id");
    }
}
