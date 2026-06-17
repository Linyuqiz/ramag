//! 剪贴板历史 CRUD。整条 ClipItem JSON 经 Cipher 加密为 hex 落主表（preview / 来源也敏感）。
//!
//! 为支撑 100 万级历史不退化，另建三张**明文**索引表，让取最近 N / 去重 / 分页
//! 都降到 O(log N) 或 O(N_可见)，不再全表解密：
//! - `clip_by_time`：key=recency_key（越新越小，见 `recency_key`），value=uuid —— 取最近 N
//! - `clip_by_hash`：key=content_hash，value=uuid —— 指纹去重 O(log N)
//! - `clip_uuid_meta`：key=uuid，value="recency_key\thash" —— 更新/删除时反查清旧索引

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use parking_lot::RwLock;
use redb::{
    Database, ReadableDatabase as _, ReadableTable, ReadableTableMetadata, TableDefinition,
};
use tracing::{debug, info};

use ramag_domain::entities::ClipItem;
use ramag_domain::error::{DomainError, Result};

use crate::encryption::Cipher;

/// 主表：key=ClipId UUID，value=加密 JSON（hex）
pub(crate) const CLIPS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("clips");
/// 时间索引：key=recency_key，value=uuid（按最近优先有序）
pub(crate) const CLIP_BY_TIME: TableDefinition<&str, &str> = TableDefinition::new("clip_by_time");
/// 去重索引：key=content_hash，value=uuid
const CLIP_BY_HASH: TableDefinition<&str, &str> = TableDefinition::new("clip_by_hash");
/// 反查表：key=uuid，value="recency_key\thash"（更新/删除时定位旧索引项）
const CLIP_UUID_META: TableDefinition<&str, &str> = TableDefinition::new("clip_uuid_meta");

fn store_err(e: impl std::fmt::Display) -> DomainError {
    DomainError::Storage(e.to_string())
}

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

/// 最近优先的有序 key：`{inverted_millis:016x}:{uuid}`。
/// inverted = u64::MAX - last_used_millis → 越新越小 → 升序遍历即最近优先；
/// 拼 uuid 保证同毫秒多条不冲突。定长 16 位 hex 让字典序 == 数值序。
fn recency_key(last_used: DateTime<Utc>, uuid: &str) -> String {
    let millis = last_used.timestamp_millis().max(0) as u64;
    let inverted = u64::MAX - millis;
    format!("{inverted:016x}:{uuid}")
}

/// 从 recency_key 反解出 last_used 毫秒（prune 判超龄用，无需解密条目）
fn millis_from_recency_key(rk: &str) -> Option<i64> {
    let hex = rk.split(':').next()?;
    let inverted = u64::from_str_radix(hex, 16).ok()?;
    Some((u64::MAX - inverted) as i64)
}

fn encode_meta(rk: &str, hash: &str) -> String {
    format!("{rk}\t{hash}")
}

fn decode_meta(s: &str) -> Option<(&str, &str)> {
    s.split_once('\t')
}

/// 全表解密（仅 clear / cleanup 等低频全量场景用，不在采集 / 唤起热路径）
fn load_all(db: &Arc<Database>, cipher: &Cipher) -> Result<Vec<ClipItem>> {
    let read_txn = db.begin_read().map_err(store_err)?;
    let table = read_txn.open_table(CLIPS_TABLE).map_err(store_err)?;
    let mut out = Vec::new();
    for entry in table.iter().map_err(store_err)? {
        let (_, v) = entry.map_err(store_err)?;
        out.push(decode(v.value(), cipher)?);
    }
    Ok(out)
}

pub(crate) fn save(db: Arc<Database>, cipher: Arc<RwLock<Cipher>>, item: ClipItem) -> Result<()> {
    let enc = {
        let cipher = cipher.read();
        encode(&item, &cipher)?
    };
    let uuid = item.id.to_string();
    let hash = item.content_hash.clone();
    let rk = recency_key(item.last_used_at, &uuid);

    let write_txn = db.begin_write().map_err(store_err)?;
    {
        let mut clips = write_txn.open_table(CLIPS_TABLE).map_err(store_err)?;
        let mut by_time = write_txn.open_table(CLIP_BY_TIME).map_err(store_err)?;
        let mut by_hash = write_txn.open_table(CLIP_BY_HASH).map_err(store_err)?;
        let mut meta = write_txn.open_table(CLIP_UUID_META).map_err(store_err)?;

        // 已存在（更新 last_used）→ 删旧时间索引项（recency_key 已变）
        let old_rk: Option<String> = meta
            .get(uuid.as_str())
            .map_err(store_err)?
            .and_then(|g| decode_meta(g.value()).map(|(rk, _)| rk.to_string()));
        if let Some(old_rk) = old_rk {
            by_time.remove(old_rk.as_str()).map_err(store_err)?;
        }

        clips
            .insert(uuid.as_str(), enc.as_str())
            .map_err(store_err)?;
        by_time
            .insert(rk.as_str(), uuid.as_str())
            .map_err(store_err)?;
        by_hash
            .insert(hash.as_str(), uuid.as_str())
            .map_err(store_err)?;
        let meta_val = encode_meta(&rk, &hash);
        meta.insert(uuid.as_str(), meta_val.as_str())
            .map_err(store_err)?;
    }
    write_txn.commit().map_err(store_err)?;
    debug!(clip_id = %uuid, "clip saved");
    Ok(())
}

/// 全量列表（按 last_used desc）。仅 cleanup_orphans 等全量场景用；日常用 `list_recent`
pub(crate) fn list(db: Arc<Database>, cipher: Arc<RwLock<Cipher>>) -> Result<Vec<ClipItem>> {
    let cipher = cipher.read();
    let mut out = load_all(&db, &cipher)?;
    out.sort_by_key(|i| std::cmp::Reverse(i.last_used_at));
    Ok(out)
}

/// 取最近 limit 条：扫时间索引前 limit 个（已最近优先），只解密这 limit 条。O(limit)
pub(crate) fn list_recent(
    db: Arc<Database>,
    cipher: Arc<RwLock<Cipher>>,
    limit: usize,
) -> Result<Vec<ClipItem>> {
    let cipher = cipher.read();
    let read_txn = db.begin_read().map_err(store_err)?;
    let by_time = read_txn.open_table(CLIP_BY_TIME).map_err(store_err)?;
    let clips = read_txn.open_table(CLIPS_TABLE).map_err(store_err)?;
    let mut out = Vec::new();
    for entry in by_time.iter().map_err(store_err)?.take(limit) {
        let (_, uuid_g) = entry.map_err(store_err)?;
        if let Some(enc_g) = clips.get(uuid_g.value()).map_err(store_err)? {
            out.push(decode(enc_g.value(), &cipher)?);
        }
    }
    Ok(out)
}

/// 全量搜索：时间索引最近优先遍历，解密匹配 query（preview/text，大小写不敏感），到 limit 停。
/// 早停让"最近匹配"快；罕见词最坏 O(N) 解密，但在后台、仅主动搜索时触发
pub(crate) fn search(
    db: Arc<Database>,
    cipher: Arc<RwLock<Cipher>>,
    query: String,
    limit: usize,
) -> Result<Vec<ClipItem>> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let cipher = cipher.read();
    let read_txn = db.begin_read().map_err(store_err)?;
    let by_time = read_txn.open_table(CLIP_BY_TIME).map_err(store_err)?;
    let clips = read_txn.open_table(CLIPS_TABLE).map_err(store_err)?;
    let mut out = Vec::new();
    for entry in by_time.iter().map_err(store_err)? {
        if out.len() >= limit {
            break;
        }
        let (_, uuid_g) = entry.map_err(store_err)?;
        if let Some(enc_g) = clips.get(uuid_g.value()).map_err(store_err)? {
            let item = decode(enc_g.value(), &cipher)?;
            let hit = item.preview.to_lowercase().contains(&q)
                || item
                    .text
                    .as_deref()
                    .is_some_and(|t| t.to_lowercase().contains(&q));
            if hit {
                out.push(item);
            }
        }
    }
    Ok(out)
}

pub(crate) fn delete(db: Arc<Database>, id: String) -> Result<()> {
    let write_txn = db.begin_write().map_err(store_err)?;
    {
        let mut clips = write_txn.open_table(CLIPS_TABLE).map_err(store_err)?;
        let mut by_time = write_txn.open_table(CLIP_BY_TIME).map_err(store_err)?;
        let mut by_hash = write_txn.open_table(CLIP_BY_HASH).map_err(store_err)?;
        let mut meta = write_txn.open_table(CLIP_UUID_META).map_err(store_err)?;

        let info: Option<(String, String)> = meta
            .get(id.as_str())
            .map_err(store_err)?
            .and_then(|g| decode_meta(g.value()).map(|(rk, h)| (rk.to_string(), h.to_string())));
        if let Some((rk, hash)) = info {
            by_time.remove(rk.as_str()).map_err(store_err)?;
            by_hash.remove(hash.as_str()).map_err(store_err)?;
        }
        meta.remove(id.as_str()).map_err(store_err)?;
        clips.remove(id.as_str()).map_err(store_err)?;
    }
    write_txn.commit().map_err(store_err)?;
    debug!(clip_id = %id, "clip deleted");
    Ok(())
}

/// 内容指纹查重：查去重索引拿 uuid → 解密该一条。O(log N)，不全表解密
pub(crate) fn find_by_hash(
    db: Arc<Database>,
    cipher: Arc<RwLock<Cipher>>,
    hash: String,
) -> Result<Option<ClipItem>> {
    let cipher = cipher.read();
    let read_txn = db.begin_read().map_err(store_err)?;
    let by_hash = read_txn.open_table(CLIP_BY_HASH).map_err(store_err)?;
    let Some(uuid_g) = by_hash.get(hash.as_str()).map_err(store_err)? else {
        return Ok(None);
    };
    let uuid = uuid_g.value().to_string();
    let clips = read_txn.open_table(CLIPS_TABLE).map_err(store_err)?;
    match clips.get(uuid.as_str()).map_err(store_err)? {
        Some(enc_g) => Ok(Some(decode(enc_g.value(), &cipher)?)),
        None => Ok(None),
    }
}

/// 清空全部历史。返回被删条目的媒体路径（调用方负责删落盘文件）
pub(crate) fn clear(db: Arc<Database>, cipher: Arc<RwLock<Cipher>>) -> Result<Vec<String>> {
    let images: Vec<String> = {
        let cipher = cipher.read();
        load_all(&db, &cipher)?
            .into_iter()
            .flat_map(|i| [i.image_path, i.thumb_path])
            .flatten()
            .collect()
    };
    let write_txn = db.begin_write().map_err(store_err)?;
    write_txn.delete_table(CLIPS_TABLE).map_err(store_err)?;
    write_txn.delete_table(CLIP_BY_TIME).map_err(store_err)?;
    write_txn.delete_table(CLIP_BY_HASH).map_err(store_err)?;
    write_txn.delete_table(CLIP_UUID_META).map_err(store_err)?;
    ensure_table(&write_txn)?;
    write_txn.commit().map_err(store_err)?;
    info!("clips cleared");
    Ok(images)
}

/// 超量 / 过期清理：扫时间索引（不解密）定位越界 / 超龄条目，只解密待删的取媒体路径
pub(crate) fn prune(
    db: Arc<Database>,
    cipher: Arc<RwLock<Cipher>>,
    max_items: u32,
    max_age_days: u32,
) -> Result<Vec<String>> {
    let cutoff_millis = (Utc::now() - Duration::days(i64::from(max_age_days))).timestamp_millis();

    // 快速路径：未超量 + 最旧未超龄 → 无需清理，避免正常采集每次都全表扫描索引
    {
        let read_txn = db.begin_read().map_err(store_err)?;
        let by_time = read_txn.open_table(CLIP_BY_TIME).map_err(store_err)?;
        let total = by_time.len().map_err(store_err)?;
        let oldest_over_age = match by_time.iter().map_err(store_err)?.next_back() {
            Some(entry) => {
                let (rk_g, _) = entry.map_err(store_err)?;
                millis_from_recency_key(rk_g.value()).is_some_and(|m| m < cutoff_millis)
            }
            None => false,
        };
        if total <= u64::from(max_items) && !oldest_over_age {
            return Ok(Vec::new());
        }
    }

    let doomed: Vec<String> = {
        let read_txn = db.begin_read().map_err(store_err)?;
        let by_time = read_txn.open_table(CLIP_BY_TIME).map_err(store_err)?;
        let mut doomed = Vec::new();
        for (idx, entry) in by_time.iter().map_err(store_err)?.enumerate() {
            let (rk_g, uuid_g) = entry.map_err(store_err)?;
            let over_count = idx >= max_items as usize;
            let over_age = millis_from_recency_key(rk_g.value()).is_some_and(|m| m < cutoff_millis);
            if over_count || over_age {
                doomed.push(uuid_g.value().to_string());
            }
        }
        doomed
    };
    if doomed.is_empty() {
        return Ok(Vec::new());
    }

    // 只解密待删条目取媒体路径（数量有限，非全表）
    let images: Vec<String> = {
        let cipher = cipher.read();
        let read_txn = db.begin_read().map_err(store_err)?;
        let clips = read_txn.open_table(CLIPS_TABLE).map_err(store_err)?;
        let mut imgs = Vec::new();
        for uuid in &doomed {
            if let Some(g) = clips.get(uuid.as_str()).map_err(store_err)? {
                let item = decode(g.value(), &cipher)?;
                imgs.extend([item.image_path, item.thumb_path].into_iter().flatten());
            }
        }
        imgs
    };

    let write_txn = db.begin_write().map_err(store_err)?;
    {
        let mut clips = write_txn.open_table(CLIPS_TABLE).map_err(store_err)?;
        let mut by_time = write_txn.open_table(CLIP_BY_TIME).map_err(store_err)?;
        let mut by_hash = write_txn.open_table(CLIP_BY_HASH).map_err(store_err)?;
        let mut meta = write_txn.open_table(CLIP_UUID_META).map_err(store_err)?;
        for uuid in &doomed {
            let info: Option<(String, String)> =
                meta.get(uuid.as_str()).map_err(store_err)?.and_then(|g| {
                    decode_meta(g.value()).map(|(rk, h)| (rk.to_string(), h.to_string()))
                });
            if let Some((rk, hash)) = info {
                by_time.remove(rk.as_str()).map_err(store_err)?;
                by_hash.remove(hash.as_str()).map_err(store_err)?;
            }
            meta.remove(uuid.as_str()).map_err(store_err)?;
            clips.remove(uuid.as_str()).map_err(store_err)?;
        }
    }
    write_txn.commit().map_err(store_err)?;
    info!(
        removed = doomed.len(),
        max_items, max_age_days, "clips pruned"
    );
    Ok(images)
}

/// 由 lib.rs 在 open 时调：建主表 + 三张索引表
pub(crate) fn ensure_table(write_txn: &redb::WriteTransaction) -> Result<()> {
    write_txn.open_table(CLIPS_TABLE).map_err(store_err)?;
    write_txn.open_table(CLIP_BY_TIME).map_err(store_err)?;
    write_txn.open_table(CLIP_BY_HASH).map_err(store_err)?;
    write_txn.open_table(CLIP_UUID_META).map_err(store_err)?;
    Ok(())
}

/// 首启迁移：主表非空但时间索引为空（旧版本数据 / 索引缺失）→ 解密全部重建三索引。
/// 一次性，之后索引随写操作在线维护
pub(crate) fn migrate_indexes(db: Arc<Database>, cipher: Arc<RwLock<Cipher>>) -> Result<()> {
    let need = {
        let read_txn = db.begin_read().map_err(store_err)?;
        let clips = read_txn.open_table(CLIPS_TABLE).map_err(store_err)?;
        let by_time = read_txn.open_table(CLIP_BY_TIME).map_err(store_err)?;
        clips.len().map_err(store_err)? > 0 && by_time.len().map_err(store_err)? == 0
    };
    if !need {
        return Ok(());
    }

    let cipher = cipher.read();
    // 先收集（解密）再写索引，避免同表借用冲突
    let entries: Vec<(String, String, String)> = {
        let read_txn = db.begin_read().map_err(store_err)?;
        let clips = read_txn.open_table(CLIPS_TABLE).map_err(store_err)?;
        let mut v = Vec::new();
        for entry in clips.iter().map_err(store_err)? {
            let (k, val) = entry.map_err(store_err)?;
            let item = decode(val.value(), &cipher)?;
            let rk = recency_key(item.last_used_at, k.value());
            v.push((k.value().to_string(), rk, item.content_hash));
        }
        v
    };

    let write_txn = db.begin_write().map_err(store_err)?;
    {
        let mut by_time = write_txn.open_table(CLIP_BY_TIME).map_err(store_err)?;
        let mut by_hash = write_txn.open_table(CLIP_BY_HASH).map_err(store_err)?;
        let mut meta = write_txn.open_table(CLIP_UUID_META).map_err(store_err)?;
        for (uuid, rk, hash) in &entries {
            by_time
                .insert(rk.as_str(), uuid.as_str())
                .map_err(store_err)?;
            by_hash
                .insert(hash.as_str(), uuid.as_str())
                .map_err(store_err)?;
            meta.insert(uuid.as_str(), encode_meta(rk, hash).as_str())
                .map_err(store_err)?;
        }
    }
    write_txn.commit().map_err(store_err)?;
    info!(count = entries.len(), "clip indexes migrated");
    Ok(())
}
