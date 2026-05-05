//! 通用偏好 KV CRUD（同步内部实现）
//!
//! 用途：主题模式 / 上次连接 ID / 窗口尺寸等单条 string 偏好

use std::sync::Arc;

use redb::{Database, ReadableDatabase as _, TableDefinition};

use ramag_domain::error::{DomainError, Result};

pub(crate) const PREFERENCES_TABLE: TableDefinition<&str, &str> =
    TableDefinition::new("preferences");

pub(crate) fn get(db: Arc<Database>, key: String) -> Result<Option<String>> {
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
}

pub(crate) fn set(db: Arc<Database>, key: String, value: String) -> Result<()> {
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
}
