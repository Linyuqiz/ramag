//! 剪贴板历史 CRUD。整条 ClipItem JSON 经 Cipher 加密为 hex 落盘（preview / 来源也属敏感内容）

use std::sync::Arc;

use chrono::{Duration, Utc};
use parking_lot::RwLock;
use redb::{Database, ReadableDatabase as _, ReadableTable, TableDefinition};
use tracing::{debug, info};

use ramag_domain::entities::ClipItem;
use ramag_domain::error::{DomainError, Result};

use crate::encryption::Cipher;

/// key=ClipId UUID，value=加密 JSON（hex）
pub(crate) const CLIPS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("clips");

fn encode(item: &ClipItem, cipher: &Cipher) -> Result<String> {
    let json = serde_json::to_string(item)
        .map_err(|e| DomainError::Storage(format!("序列化剪贴条目失败：{e}")))?;
    cipher.encrypt(&json)
}

fn decode(hex: &str, cipher: &Cipher) -> Result<ClipItem> {
    let json = cipher.decrypt(hex)?;
    serde_json::from_str(&json)
        .map_err(|e| DomainError::Storage(format!("反序列化剪贴条目失败：{e}")))
}

fn load_all(db: &Arc<Database>, cipher: &Cipher) -> Result<Vec<ClipItem>> {
    let read_txn = db
        .begin_read()
        .map_err(|e| DomainError::Storage(format!("启动读事务失败：{e}")))?;
    let table = read_txn
        .open_table(CLIPS_TABLE)
        .map_err(|e| DomainError::Storage(format!("打开 clips 表失败：{e}")))?;
    let mut out = Vec::new();
    for entry in table
        .iter()
        .map_err(|e| DomainError::Storage(e.to_string()))?
    {
        let (_, v) = entry.map_err(|e| DomainError::Storage(e.to_string()))?;
        out.push(decode(v.value(), cipher)?);
    }
    Ok(out)
}

fn remove_ids(db: &Arc<Database>, ids: &[String]) -> Result<()> {
    let write_txn = db
        .begin_write()
        .map_err(|e| DomainError::Storage(format!("启动写事务失败：{e}")))?;
    {
        let mut table = write_txn
            .open_table(CLIPS_TABLE)
            .map_err(|e| DomainError::Storage(format!("打开 clips 表失败：{e}")))?;
        for id in ids {
            table
                .remove(id.as_str())
                .map_err(|e| DomainError::Storage(format!("删除剪贴条目失败：{e}")))?;
        }
    }
    write_txn
        .commit()
        .map_err(|e| DomainError::Storage(format!("提交事务失败：{e}")))?;
    Ok(())
}

pub(crate) fn save(db: Arc<Database>, cipher: Arc<RwLock<Cipher>>, item: ClipItem) -> Result<()> {
    let enc = {
        let cipher = cipher.read();
        encode(&item, &cipher)?
    };
    let id_str = item.id.to_string();
    let write_txn = db
        .begin_write()
        .map_err(|e| DomainError::Storage(format!("启动写事务失败：{e}")))?;
    {
        let mut table = write_txn
            .open_table(CLIPS_TABLE)
            .map_err(|e| DomainError::Storage(format!("打开 clips 表失败：{e}")))?;
        table
            .insert(id_str.as_str(), enc.as_str())
            .map_err(|e| DomainError::Storage(format!("写入剪贴条目失败：{e}")))?;
    }
    write_txn
        .commit()
        .map_err(|e| DomainError::Storage(format!("提交事务失败：{e}")))?;
    debug!(clip_id = %id_str, "clip saved");
    Ok(())
}

/// 按 last_used_at desc
pub(crate) fn list(db: Arc<Database>, cipher: Arc<RwLock<Cipher>>) -> Result<Vec<ClipItem>> {
    let cipher = cipher.read();
    let mut out = load_all(&db, &cipher)?;
    out.sort_by_key(|i| std::cmp::Reverse(i.last_used_at));
    Ok(out)
}

pub(crate) fn delete(db: Arc<Database>, id: String) -> Result<()> {
    remove_ids(&db, std::slice::from_ref(&id))?;
    debug!(clip_id = %id, "clip deleted");
    Ok(())
}

/// 内容指纹查重（全表扫描；量级受 max_items 约束）
pub(crate) fn find_by_hash(
    db: Arc<Database>,
    cipher: Arc<RwLock<Cipher>>,
    hash: String,
) -> Result<Option<ClipItem>> {
    let cipher = cipher.read();
    Ok(load_all(&db, &cipher)?
        .into_iter()
        .find(|i| i.content_hash == hash))
}

/// 清空全部历史。返回被删条目的 image_path（调用方负责删落盘文件）
pub(crate) fn clear(db: Arc<Database>, cipher: Arc<RwLock<Cipher>>) -> Result<Vec<String>> {
    let all = {
        let cipher = cipher.read();
        load_all(&db, &cipher)?
    };
    let ids: Vec<String> = all.iter().map(|i| i.id.to_string()).collect();
    let images: Vec<String> = all
        .iter()
        .flat_map(|i| [i.image_path.clone(), i.thumb_path.clone()])
        .flatten()
        .collect();
    remove_ids(&db, &ids)?;
    info!(removed = ids.len(), "clips cleared");
    Ok(images)
}

/// 超量 / 过期清理：按 last_used_at desc 保留前 max_items 条，且剔除超龄项
pub(crate) fn prune(
    db: Arc<Database>,
    cipher: Arc<RwLock<Cipher>>,
    max_items: u32,
    max_age_days: u32,
) -> Result<Vec<String>> {
    let all = {
        let cipher = cipher.read();
        load_all(&db, &cipher)?
    };
    let cutoff = Utc::now() - Duration::days(i64::from(max_age_days));
    let mut sorted: Vec<&ClipItem> = all.iter().collect();
    sorted.sort_by_key(|i| std::cmp::Reverse(i.last_used_at));

    let mut doomed: Vec<&ClipItem> = Vec::new();
    for (idx, item) in sorted.iter().enumerate() {
        if idx >= max_items as usize || item.last_used_at < cutoff {
            doomed.push(item);
        }
    }
    if doomed.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = doomed.iter().map(|i| i.id.to_string()).collect();
    let images: Vec<String> = doomed
        .iter()
        .flat_map(|i| [i.image_path.clone(), i.thumb_path.clone()])
        .flatten()
        .collect();
    remove_ids(&db, &ids)?;
    info!(removed = ids.len(), max_items, max_age_days, "clips pruned");
    Ok(images)
}

/// 由 lib.rs 在 open 时调
pub(crate) fn ensure_table(write_txn: &redb::WriteTransaction) -> Result<()> {
    let _ = write_txn
        .open_table(CLIPS_TABLE)
        .map_err(|e| DomainError::Storage(format!("打开 clips 表失败：{e}")))?;
    Ok(())
}
