//! 连接配置 CRUD。密码经 Cipher 加密为 hex 落盘；密钥变化 / 数据损坏读取时抛 Storage 错

use std::sync::Arc;

use parking_lot::RwLock;
use redb::{Database, ReadableDatabase as _, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use ramag_domain::entities::{ConnectionConfig, ConnectionId};
use ramag_domain::error::{DomainError, Result};

use crate::encryption::Cipher;

/// key=ConnectionId UUID，value=`EncryptedConnection` JSON
pub(crate) const CONNECTIONS_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("connections");

/// 落盘版连接，密码加密为 hex
#[derive(Debug, Serialize, Deserialize)]
struct EncryptedConnection {
    id: ConnectionId,
    name: String,
    driver: ramag_domain::entities::DriverKind,
    host: String,
    port: u16,
    username: String,
    /// 加密密码（hex）；明文不入库
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

pub(crate) fn list(
    db: Arc<Database>,
    cipher: Arc<RwLock<Cipher>>,
) -> Result<Vec<ConnectionConfig>> {
    let read_txn = db
        .begin_read()
        .map_err(|e| DomainError::Storage(format!("启动读事务失败：{e}")))?;
    let table = read_txn
        .open_table(CONNECTIONS_TABLE)
        .map_err(|e| DomainError::Storage(format!("打开表失败：{e}")))?;

    let cipher = cipher.read();
    let mut out = Vec::new();
    for entry in table
        .iter()
        .map_err(|e| DomainError::Storage(e.to_string()))?
    {
        let (_, v) = entry.map_err(|e| DomainError::Storage(e.to_string()))?;
        let enc: EncryptedConnection = serde_json::from_str(v.value())
            .map_err(|e| DomainError::Storage(format!("反序列化连接失败：{e}")))?;
        out.push(enc.into_plain(&cipher)?);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    debug!(count = out.len(), "list_connections done");
    Ok(out)
}

pub(crate) fn get(
    db: Arc<Database>,
    cipher: Arc<RwLock<Cipher>>,
    id: String,
) -> Result<Option<ConnectionConfig>> {
    let read_txn = db
        .begin_read()
        .map_err(|e| DomainError::Storage(format!("启动读事务失败：{e}")))?;
    let table = read_txn
        .open_table(CONNECTIONS_TABLE)
        .map_err(|e| DomainError::Storage(format!("打开表失败：{e}")))?;

    match table
        .get(id.as_str())
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
}

pub(crate) fn save(
    db: Arc<Database>,
    cipher: Arc<RwLock<Cipher>>,
    config: ConnectionConfig,
) -> Result<()> {
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
}

pub(crate) fn delete(db: Arc<Database>, id: String) -> Result<()> {
    let write_txn = db
        .begin_write()
        .map_err(|e| DomainError::Storage(format!("启动写事务失败：{e}")))?;
    {
        let mut table = write_txn
            .open_table(CONNECTIONS_TABLE)
            .map_err(|e| DomainError::Storage(format!("打开表失败：{e}")))?;
        table
            .remove(id.as_str())
            .map_err(|e| DomainError::Storage(format!("删除失败：{e}")))?;
    }
    write_txn
        .commit()
        .map_err(|e| DomainError::Storage(format!("提交事务失败：{e}")))?;

    info!(connection_id = %id, "connection deleted");
    Ok(())
}

/// 由 lib.rs 在 open 时调
pub(crate) fn ensure_table(write_txn: &redb::WriteTransaction) -> Result<()> {
    let _ = write_txn
        .open_table(CONNECTIONS_TABLE)
        .map_err(|e| DomainError::Storage(format!("打开 connections 表失败：{e}")))?;
    Ok(())
}
