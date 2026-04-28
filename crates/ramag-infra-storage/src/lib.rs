//! Ramag 本地存储实现
//!
//! 用 redb 嵌入式数据库存连接配置 / 查询历史 / 收藏夹。
//! 敏感字段（密码）用 aes-gcm 加密，主密钥存 macOS 钥匙串。
//!
//! # 文件位置
//!
//! `~/Library/Application Support/com.ramag.ramag/ramag.redb`（macOS）
//! `~/.local/share/ramag/ramag.redb`（Linux）
//!
//! # 用法
//!
//! ```no_run
//! use std::sync::Arc;
//! use ramag_domain::traits::Storage;
//! use ramag_infra_storage::RedbStorage;
//!
//! # async fn demo() -> ramag_domain::error::Result<()> {
//! let storage: Arc<dyn Storage> = Arc::new(RedbStorage::open_default()?);
//! let connections = storage.list_connections().await?;
//! # Ok(()) }
//! ```

pub mod encryption;
pub mod keyring;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use directories::ProjectDirs;
use futures::channel::oneshot;
use parking_lot::RwLock;
use redb::{Database, ReadableDatabase as _, ReadableTable, ReadableTableMetadata as _, TableDefinition};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use ramag_domain::entities::{ConnectionConfig, ConnectionId, QueryRecord, QueryRecordId};
use ramag_domain::error::{DomainError, Result};
use ramag_domain::traits::Storage;

use crate::encryption::Cipher;

/// redb 表定义：连接配置
///
/// key: ConnectionId（UUID 字符串）
/// value: JSON 序列化后的 EncryptedConnection
const CONNECTIONS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("connections");

/// 查询历史表
///
/// key: 形如 `{rfc3339_timestamp}_{record_id}` 的复合 key
///   - 时间戳前缀让 redb 按时间有序遍历
///   - record_id 后缀避免毫秒级冲突
/// value: JSON 序列化的 QueryRecord
const HISTORY_TABLE: TableDefinition<&str, &str> = TableDefinition::new("query_history");

/// 历史保留上限：超过自动裁剪最旧的（防止无限增长）
const HISTORY_MAX_KEEP: usize = 5000;

/// 通用偏好 KV 表（如主题模式 / 上次连接 ID 等）
const PREFERENCES_TABLE: TableDefinition<&str, &str> = TableDefinition::new("preferences");

/// 加密后落盘的连接配置
///
/// 密码字段被加密成 hex 字符串
#[derive(Debug, Serialize, Deserialize)]
struct EncryptedConnection {
    id: ConnectionId,
    name: String,
    driver: ramag_domain::entities::DriverKind,
    host: String,
    port: u16,
    username: String,
    /// 加密的密码（hex 字符串），明文不入库
    password_enc: String,
    database: Option<String>,
    remark: Option<String>,
    #[serde(default)]
    color: ramag_domain::entities::ConnectionColor,
}

impl EncryptedConnection {
    fn from_plain(plain: &ConnectionConfig, cipher: &Cipher) -> Result<Self> {
        Ok(Self {
            id: plain.id.clone(),
            name: plain.name.clone(),
            driver: plain.driver,
            host: plain.host.clone(),
            port: plain.port,
            username: plain.username.clone(),
            password_enc: cipher.encrypt(&plain.password)?,
            database: plain.database.clone(),
            remark: plain.remark.clone(),
            color: plain.color,
        })
    }

    fn into_plain(self, cipher: &Cipher) -> Result<ConnectionConfig> {
        Ok(ConnectionConfig {
            id: self.id,
            name: self.name,
            driver: self.driver,
            host: self.host,
            port: self.port,
            username: self.username,
            password: cipher.decrypt(&self.password_enc)?,
            database: self.database,
            remark: self.remark,
            color: self.color,
        })
    }
}

/// 基于 redb 的本地存储实现
pub struct RedbStorage {
    db: Arc<Database>,
    cipher: Arc<RwLock<Cipher>>,
    /// 数据库文件路径（用于调试 / 重置）
    path: PathBuf,
}

impl RedbStorage {
    /// 用默认路径打开（首次会创建文件 + 钥匙串里生成主密钥）
    pub fn open_default() -> Result<Self> {
        let path = default_db_path()?;
        Self::open(&path)
    }

    /// 在指定路径打开 / 创建数据库（生产模式：从系统钥匙串读主密钥）
    pub fn open(path: &Path) -> Result<Self> {
        let master_key = keyring::get_or_create_master_key()?;
        Self::open_with_key(path, &master_key)
    }

    /// 用显式给定的主密钥打开数据库（测试模式 / 高级用法）
    ///
    /// 单元测试使用此入口，避免污染真实钥匙串
    pub fn open_with_key(path: &Path, master_key: &[u8; 32]) -> Result<Self> {
        // 确保父目录存在
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| DomainError::Storage(format!("创建数据目录失败：{e}")))?;
        }

        let db = Database::create(path)
            .map_err(|e| DomainError::Storage(format!("打开 redb 数据库失败：{e}")))?;

        // 首次打开数据库时建表（commit 一次空写入事务）
        let write_txn = db
            .begin_write()
            .map_err(|e| DomainError::Storage(format!("启动写事务失败：{e}")))?;
        {
            let _ = write_txn
                .open_table(CONNECTIONS_TABLE)
                .map_err(|e| DomainError::Storage(format!("打开 connections 表失败：{e}")))?;
            let _ = write_txn
                .open_table(HISTORY_TABLE)
                .map_err(|e| DomainError::Storage(format!("打开 history 表失败：{e}")))?;
        }
        write_txn
            .commit()
            .map_err(|e| DomainError::Storage(format!("提交事务失败：{e}")))?;

        let cipher = Cipher::new(master_key);

        info!(path = %path.display(), "redb storage opened");

        Ok(Self {
            db: Arc::new(db),
            cipher: Arc::new(RwLock::new(cipher)),
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// 计算默认数据库文件路径
fn default_db_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("com", "ramag", "ramag")
        .ok_or_else(|| DomainError::Storage("无法定位用户目录".into()))?;
    Ok(dirs.data_dir().join("ramag.redb"))
}

/// 在独立 std 线程跑同步代码，结果通过 oneshot 送回
///
/// 用 std::thread + futures::oneshot 而不是 tokio::task::spawn_blocking，
/// 这样无论调用方在 tokio / smol / async-std 哪种 runtime 下都能用。
async fn run_blocking<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        let result = f();
        let _ = tx.send(result);
    });
    rx.await
        .unwrap_or_else(|_| Err(DomainError::Storage("storage worker 线程异常退出".into())))
}

#[async_trait]
impl Storage for RedbStorage {
    async fn list_connections(&self) -> Result<Vec<ConnectionConfig>> {
        let db = self.db.clone();
        let cipher = self.cipher.clone();

        run_blocking(move || {
            let read_txn = db
                .begin_read()
                .map_err(|e| DomainError::Storage(format!("启动读事务失败：{e}")))?;
            let table = read_txn
                .open_table(CONNECTIONS_TABLE)
                .map_err(|e| DomainError::Storage(format!("打开表失败：{e}")))?;

            let cipher = cipher.read();
            let mut out = Vec::new();
            for entry in table.iter().map_err(|e| DomainError::Storage(e.to_string()))? {
                let (_, v) = entry.map_err(|e| DomainError::Storage(e.to_string()))?;
                let enc: EncryptedConnection = serde_json::from_str(v.value())
                    .map_err(|e| DomainError::Storage(format!("反序列化连接失败：{e}")))?;
                out.push(enc.into_plain(&cipher)?);
            }
            out.sort_by(|a, b| a.name.cmp(&b.name));
            debug!(count = out.len(), "list_connections done");
            Ok(out)
        })
        .await
    }

    async fn get_connection(&self, id: &ConnectionId) -> Result<Option<ConnectionConfig>> {
        let db = self.db.clone();
        let cipher = self.cipher.clone();
        let id_str = id.to_string();

        run_blocking(move || {
            let read_txn = db
                .begin_read()
                .map_err(|e| DomainError::Storage(format!("启动读事务失败：{e}")))?;
            let table = read_txn
                .open_table(CONNECTIONS_TABLE)
                .map_err(|e| DomainError::Storage(format!("打开表失败：{e}")))?;

            match table
                .get(id_str.as_str())
                .map_err(|e| DomainError::Storage(e.to_string()))?
            {
                Some(v) => {
                    let cipher = cipher.read();
                    let enc: EncryptedConnection = serde_json::from_str(v.value())
                        .map_err(|e| DomainError::Storage(format!("反序列化失败：{e}")))?;
                    Ok(Some(enc.into_plain(&cipher)?))
                }
                None => Ok(None),
            }
        })
        .await
    }

    async fn save_connection(&self, config: &ConnectionConfig) -> Result<()> {
        let db = self.db.clone();
        let cipher = self.cipher.clone();
        let config = config.clone();

        run_blocking(move || {
            let cipher = cipher.read();
            let enc = EncryptedConnection::from_plain(&config, &cipher)?;
            let json = serde_json::to_string(&enc)
                .map_err(|e| DomainError::Storage(format!("序列化失败：{e}")))?;
            let id_str = config.id.to_string();

            let write_txn = db
                .begin_write()
                .map_err(|e| DomainError::Storage(format!("启动写事务失败：{e}")))?;
            {
                let mut table = write_txn
                    .open_table(CONNECTIONS_TABLE)
                    .map_err(|e| DomainError::Storage(format!("打开表失败：{e}")))?;
                table
                    .insert(id_str.as_str(), json.as_str())
                    .map_err(|e| DomainError::Storage(format!("写入失败：{e}")))?;
            }
            write_txn
                .commit()
                .map_err(|e| DomainError::Storage(format!("提交事务失败：{e}")))?;

            info!(connection_id = %config.id, name = %config.name, "connection saved");
            Ok(())
        })
        .await
    }

    async fn delete_connection(&self, id: &ConnectionId) -> Result<()> {
        let db = self.db.clone();
        let id_str = id.to_string();

        run_blocking(move || {
            let write_txn = db
                .begin_write()
                .map_err(|e| DomainError::Storage(format!("启动写事务失败：{e}")))?;
            {
                let mut table = write_txn
                    .open_table(CONNECTIONS_TABLE)
                    .map_err(|e| DomainError::Storage(format!("打开表失败：{e}")))?;
                table
                    .remove(id_str.as_str())
                    .map_err(|e| DomainError::Storage(format!("删除失败：{e}")))?;
            }
            write_txn
                .commit()
                .map_err(|e| DomainError::Storage(format!("提交事务失败：{e}")))?;

            info!(connection_id = %id_str, "connection deleted");
            Ok(())
        })
        .await
    }

    // ==================== 查询历史 ====================

    async fn append_history(&self, record: &QueryRecord) -> Result<()> {
        let db = self.db.clone();
        let record = record.clone();

        run_blocking(move || {
            // key = "{rfc3339}_{id}" → 字典序按时间升序
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

                // 超过上限：删最早的 N 条
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
        })
        .await
    }

    async fn list_history(
        &self,
        connection_id: Option<&ConnectionId>,
        limit: usize,
    ) -> Result<Vec<QueryRecord>> {
        let db = self.db.clone();
        let conn_filter = connection_id.cloned();

        run_blocking(move || {
            let read_txn = db
                .begin_read()
                .map_err(|e| DomainError::Storage(format!("启动读事务失败：{e}")))?;
            let table = read_txn
                .open_table(HISTORY_TABLE)
                .map_err(|e| DomainError::Storage(format!("打开 history 表失败：{e}")))?;

            // 倒序取（最新的在最后），limit 控制
            let mut all: Vec<QueryRecord> = Vec::new();
            for entry in table.iter().map_err(|e| DomainError::Storage(e.to_string()))? {
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
            // 按 executed_at desc 排序
            all.sort_by(|a, b| b.executed_at.cmp(&a.executed_at));
            all.truncate(limit);
            Ok(all)
        })
        .await
    }

    async fn delete_history(&self, id: &QueryRecordId) -> Result<()> {
        let db = self.db.clone();
        let target_id = id.0.to_string();

        run_blocking(move || {
            let write_txn = db
                .begin_write()
                .map_err(|e| DomainError::Storage(format!("启动写事务失败：{e}")))?;
            {
                let mut table = write_txn
                    .open_table(HISTORY_TABLE)
                    .map_err(|e| DomainError::Storage(format!("打开 history 表失败：{e}")))?;
                // 找出 key 包含 target_id 的项删除
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
        })
        .await
    }

    async fn clear_history(&self, connection_id: Option<&ConnectionId>) -> Result<()> {
        let db = self.db.clone();
        let conn_filter = connection_id.cloned();

        run_blocking(move || {
            let write_txn = db
                .begin_write()
                .map_err(|e| DomainError::Storage(format!("启动写事务失败：{e}")))?;
            {
                let mut table = write_txn
                    .open_table(HISTORY_TABLE)
                    .map_err(|e| DomainError::Storage(format!("打开 history 表失败：{e}")))?;

                if conn_filter.is_none() {
                    // 全部清空：用 retain 风格——逐个 remove
                    let all_keys: Vec<String> = table
                        .iter()
                        .map_err(|e| DomainError::Storage(e.to_string()))?
                        .filter_map(|r| r.ok().map(|(k, _)| k.value().to_string()))
                        .collect();
                    for k in all_keys {
                        let _ = table.remove(k.as_str());
                    }
                } else {
                    let target = conn_filter.unwrap();
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
                }
            }
            write_txn
                .commit()
                .map_err(|e| DomainError::Storage(format!("提交事务失败：{e}")))?;
            info!("history cleared");
            Ok(())
        })
        .await
    }

    async fn get_preference(&self, key: &str) -> Result<Option<String>> {
        let db = self.db.clone();
        let key = key.to_string();
        run_blocking(move || {
            let read_txn = db
                .begin_read()
                .map_err(|e| DomainError::Storage(format!("启动读事务失败：{e}")))?;
            let table = match read_txn.open_table(PREFERENCES_TABLE) {
                Ok(t) => t,
                Err(_) => return Ok(None), // 表不存在视为未设置
            };
            let v = table
                .get(key.as_str())
                .map_err(|e| DomainError::Storage(format!("读偏好失败：{e}")))?
                .map(|g| g.value().to_string());
            Ok(v)
        })
        .await
    }

    async fn set_preference(&self, key: &str, value: &str) -> Result<()> {
        let db = self.db.clone();
        let key = key.to_string();
        let value = value.to_string();
        run_blocking(move || {
            let write_txn = db
                .begin_write()
                .map_err(|e| DomainError::Storage(format!("启动写事务失败：{e}")))?;
            {
                let mut table = write_txn
                    .open_table(PREFERENCES_TABLE)
                    .map_err(|e| DomainError::Storage(format!("打开 preferences 表失败：{e}")))?;
                table
                    .insert(key.as_str(), value.as_str())
                    .map_err(|e| DomainError::Storage(format!("写偏好失败：{e}")))?;
            }
            write_txn
                .commit()
                .map_err(|e| DomainError::Storage(format!("提交事务失败：{e}")))?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ramag_domain::entities::DriverKind;
    use tempfile::TempDir;

    /// 创建测试用 storage（用临时文件夹 + 固定主密钥，避免污染真实钥匙串）
    fn make_test_storage() -> (RedbStorage, TempDir) {
        let tmp = TempDir::new().expect("创建临时目录失败");
        let path = tmp.path().join("test.redb");
        // 测试用固定密钥，全 0x42
        let key = [0x42u8; 32];
        let storage = RedbStorage::open_with_key(&path, &key).expect("打开测试 storage 失败");
        (storage, tmp)
    }

    fn sample_config(name: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: ConnectionId::new(),
            name: name.to_string(),
            driver: DriverKind::Mysql,
            host: "127.0.0.1".into(),
            port: 3306,
            username: "root".into(),
            password: "secret-password".into(),
            database: Some("test".into()),
            remark: None,
            color: Default::default(),
        }
    }

    #[tokio::test]
    async fn save_and_list() {
        let (storage, _tmp) = make_test_storage();
        let cfg = sample_config("dev");

        storage.save_connection(&cfg).await.unwrap();
        let list = storage.list_connections().await.unwrap();

        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "dev");
        assert_eq!(list[0].password, "secret-password"); // 解密回明文
    }

    #[tokio::test]
    async fn save_and_get_by_id() {
        let (storage, _tmp) = make_test_storage();
        let cfg = sample_config("prod");

        storage.save_connection(&cfg).await.unwrap();
        let got = storage.get_connection(&cfg.id).await.unwrap();
        assert!(got.is_some());
        assert_eq!(got.unwrap().name, "prod");
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let (storage, _tmp) = make_test_storage();
        let id = ConnectionId::new();
        let got = storage.get_connection(&id).await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn delete_works() {
        let (storage, _tmp) = make_test_storage();
        let cfg = sample_config("a");
        storage.save_connection(&cfg).await.unwrap();
        assert_eq!(storage.list_connections().await.unwrap().len(), 1);

        storage.delete_connection(&cfg.id).await.unwrap();
        assert_eq!(storage.list_connections().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn list_sorted_by_name() {
        let (storage, _tmp) = make_test_storage();
        for n in &["zebra", "apple", "mongo"] {
            storage.save_connection(&sample_config(n)).await.unwrap();
        }
        let list = storage.list_connections().await.unwrap();
        let names: Vec<_> = list.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["apple", "mongo", "zebra"]);
    }

    #[tokio::test]
    async fn update_existing() {
        let (storage, _tmp) = make_test_storage();
        let mut cfg = sample_config("dev");
        storage.save_connection(&cfg).await.unwrap();

        cfg.host = "10.0.0.1".to_string();
        storage.save_connection(&cfg).await.unwrap();

        let got = storage.get_connection(&cfg.id).await.unwrap().unwrap();
        assert_eq!(got.host, "10.0.0.1");
    }
}
