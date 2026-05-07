//! 查询历史 CRUD。复合 key=`{rfc3339}_{id}`（时间字典序 + 防毫秒撞）；超 HISTORY_MAX_KEEP 删最早

use std::sync::Arc;

use redb::{
    Database, ReadableDatabase as _, ReadableTable, ReadableTableMetadata as _, TableDefinition,
};
use tracing::{debug, info};

use ramag_domain::entities::{ConnectionId, QueryRecord, QueryRecordId};
use ramag_domain::error::{DomainError, Result};

pub(crate) const HISTORY_TABLE: TableDefinition<&str, &str> = TableDefinition::new("query_history");

const HISTORY_MAX_KEEP: usize = 5000;

pub(crate) fn append(db: Arc<Database>, record: QueryRecord) -> Result<()> {
    let key = format!("{}_{}", record.executed_at.to_rfc3339(), record.id);
    let value = serde_json::to_string(&record)
        .map_err(|e| DomainError::Storage(format!("history 序列化失败：{e}")))?;

    let write_txn = db
        .begin_write()
        .map_err(|e| DomainError::Storage(format!("启动写事务失败：{e}")))?;
    {
        let mut table = write_txn
            .open_table(HISTORY_TABLE)
            .map_err(|e| DomainError::Storage(format!("打开 history 表失败：{e}")))?;
        table
            .insert(key.as_str(), value.as_str())
            .map_err(|e| DomainError::Storage(format!("写入历史失败：{e}")))?;

        let len = table.len().unwrap_or(0) as usize;
        if len > HISTORY_MAX_KEEP {
            let to_remove = len - HISTORY_MAX_KEEP;
            let oldest_keys: Vec<String> = table
                .iter()
                .map_err(|e| DomainError::Storage(e.to_string()))?
                .take(to_remove)
                .filter_map(|r| r.ok().map(|(k, _)| k.value().to_string()))
                .collect();
            for k in oldest_keys {
                let _ = table.remove(k.as_str());
            }
        }
    }
    write_txn
        .commit()
        .map_err(|e| DomainError::Storage(format!("提交事务失败：{e}")))?;
    debug!(record_id = %record.id, "history appended");
    Ok(())
}

pub(crate) fn list(
    db: Arc<Database>,
    conn_filter: Option<ConnectionId>,
    limit: usize,
) -> Result<Vec<QueryRecord>> {
    let read_txn = db
        .begin_read()
        .map_err(|e| DomainError::Storage(format!("启动读事务失败：{e}")))?;
    let table = read_txn
        .open_table(HISTORY_TABLE)
        .map_err(|e| DomainError::Storage(format!("打开 history 表失败：{e}")))?;

    let mut all: Vec<QueryRecord> = Vec::new();
    for entry in table
        .iter()
        .map_err(|e| DomainError::Storage(e.to_string()))?
    {
        let (_, v) = entry.map_err(|e| DomainError::Storage(e.to_string()))?;
        if let Ok(rec) = serde_json::from_str::<QueryRecord>(v.value()) {
            if let Some(ref filter_id) = conn_filter
                && rec.connection_id != *filter_id
            {
                continue;
            }
            all.push(rec);
        }
    }
    all.sort_by_key(|r| std::cmp::Reverse(r.executed_at));
    all.truncate(limit);
    Ok(all)
}

pub(crate) fn delete(db: Arc<Database>, id: QueryRecordId) -> Result<()> {
    let target_id = id.0.to_string();

    let write_txn = db
        .begin_write()
        .map_err(|e| DomainError::Storage(format!("启动写事务失败：{e}")))?;
    {
        let mut table = write_txn
            .open_table(HISTORY_TABLE)
            .map_err(|e| DomainError::Storage(format!("打开 history 表失败：{e}")))?;
        // 复合 key 包含 record_id，按子串匹配删除
        let keys_to_remove: Vec<String> = table
            .iter()
            .map_err(|e| DomainError::Storage(e.to_string()))?
            .filter_map(|r| r.ok())
            .filter(|(k, _)| k.value().contains(&target_id))
            .map(|(k, _)| k.value().to_string())
            .collect();
        for k in keys_to_remove {
            let _ = table.remove(k.as_str());
        }
    }
    write_txn
        .commit()
        .map_err(|e| DomainError::Storage(format!("提交事务失败：{e}")))?;
    Ok(())
}

pub(crate) fn clear(db: Arc<Database>, conn_filter: Option<ConnectionId>) -> Result<()> {
    let write_txn = db
        .begin_write()
        .map_err(|e| DomainError::Storage(format!("启动写事务失败：{e}")))?;
    {
        let mut table = write_txn
            .open_table(HISTORY_TABLE)
            .map_err(|e| DomainError::Storage(format!("打开 history 表失败：{e}")))?;

        if let Some(target) = conn_filter {
            let to_remove: Vec<String> = table
                .iter()
                .map_err(|e| DomainError::Storage(e.to_string()))?
                .filter_map(|r| r.ok())
                .filter_map(|(k, v)| {
                    let rec: QueryRecord = serde_json::from_str(v.value()).ok()?;
                    if rec.connection_id == target {
                        Some(k.value().to_string())
                    } else {
                        None
                    }
                })
                .collect();
            for k in to_remove {
                let _ = table.remove(k.as_str());
            }
        } else {
            let all_keys: Vec<String> = table
                .iter()
                .map_err(|e| DomainError::Storage(e.to_string()))?
                .filter_map(|r| r.ok().map(|(k, _)| k.value().to_string()))
                .collect();
            for k in all_keys {
                let _ = table.remove(k.as_str());
            }
        }
    }
    write_txn
        .commit()
        .map_err(|e| DomainError::Storage(format!("提交事务失败：{e}")))?;
    info!("history cleared");
    Ok(())
}

pub(crate) fn ensure_table(write_txn: &redb::WriteTransaction) -> Result<()> {
    let _ = write_txn
        .open_table(HISTORY_TABLE)
        .map_err(|e| DomainError::Storage(format!("打开 history 表失败：{e}")))?;
    Ok(())
}
