//! Git 仓库（VCS 最近列表）CRUD（同步内部实现）
//!
//! 仓库配置无敏感字段，无需加密；按 path 去重（driver 每次 open 创建新 UUID，
//! 同物理仓库重复打开不堆积冗余记录）

use std::sync::Arc;

use redb::{Database, ReadableDatabase as _, ReadableTable, TableDefinition};
use tracing::{debug, info};

use ramag_domain::entities::{RepoConfig, RepoId};
use ramag_domain::error::{DomainError, Result};

/// redb 表定义：Git 仓库配置
///
/// key: RepoId（UUID 字符串）
/// value: JSON 序列化后的 RepoConfig（无加密）
pub(crate) const REPOS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("repos");

pub(crate) fn list(db: Arc<Database>) -> Result<Vec<RepoConfig>> {
    let read_txn = db
        .begin_read()
        .map_err(|e| DomainError::Storage(format!("启动读事务失败：{e}")))?;
    let table = read_txn
        .open_table(REPOS_TABLE)
        .map_err(|e| DomainError::Storage(format!("打开 repos 表失败：{e}")))?;

    let mut out: Vec<RepoConfig> = Vec::new();
    for entry in table
        .iter()
        .map_err(|e| DomainError::Storage(e.to_string()))?
    {
        let (_, v) = entry.map_err(|e| DomainError::Storage(e.to_string()))?;
        let cfg: RepoConfig = serde_json::from_str(v.value())
            .map_err(|e| DomainError::Storage(format!("反序列化仓库失败：{e}")))?;
        out.push(cfg);
    }
    // 按 name 字母序：列表顺序稳定，不随打开顺序漂移
    out.sort_by(|a, b| a.name.cmp(&b.name));
    debug!(count = out.len(), "list_repos done");
    Ok(out)
}

pub(crate) fn save(db: Arc<Database>, config: RepoConfig) -> Result<()> {
    let json = serde_json::to_string(&config)
        .map_err(|e| DomainError::Storage(format!("序列化仓库失败：{e}")))?;
    let id_str = config.id.to_string();
    let target_path = config.path.clone();

    let write_txn = db
        .begin_write()
        .map_err(|e| DomainError::Storage(format!("启动写事务失败：{e}")))?;
    {
        let mut table = write_txn
            .open_table(REPOS_TABLE)
            .map_err(|e| DomainError::Storage(format!("打开 repos 表失败：{e}")))?;

        // 同 path 去重：driver 每次 open_repo 创建新 UUID，重启后再打开同物理仓库
        // 会产生新 RepoId → 不去重的话每次都新增一条。这里在同一事务内先把所有
        // path 匹配的旧记录全部删掉，再插入新记录，保证同 path 仅一条
        let mut stale_keys: Vec<String> = Vec::new();
        for entry in table
            .iter()
            .map_err(|e| DomainError::Storage(e.to_string()))?
        {
            let (k, v) = entry.map_err(|e| DomainError::Storage(e.to_string()))?;
            if let Ok(cfg) = serde_json::from_str::<RepoConfig>(v.value())
                && cfg.path == target_path
                && k.value() != id_str
            {
                stale_keys.push(k.value().to_string());
            }
        }
        for k in stale_keys {
            table
                .remove(k.as_str())
                .map_err(|e| DomainError::Storage(format!("清理重复记录失败：{e}")))?;
        }

        table
            .insert(id_str.as_str(), json.as_str())
            .map_err(|e| DomainError::Storage(format!("写入仓库失败：{e}")))?;
    }
    write_txn
        .commit()
        .map_err(|e| DomainError::Storage(format!("提交事务失败：{e}")))?;

    info!(repo_id = %config.id, name = %config.name, "repo saved");
    Ok(())
}

pub(crate) fn delete(db: Arc<Database>, id: RepoId) -> Result<()> {
    let id_str = id.to_string();
    let write_txn = db
        .begin_write()
        .map_err(|e| DomainError::Storage(format!("启动写事务失败：{e}")))?;
    {
        let mut table = write_txn
            .open_table(REPOS_TABLE)
            .map_err(|e| DomainError::Storage(format!("打开 repos 表失败：{e}")))?;
        table
            .remove(id_str.as_str())
            .map_err(|e| DomainError::Storage(format!("删除仓库失败：{e}")))?;
    }
    write_txn
        .commit()
        .map_err(|e| DomainError::Storage(format!("提交事务失败：{e}")))?;

    info!(repo_id = %id_str, "repo deleted");
    Ok(())
}

pub(crate) fn ensure_table(write_txn: &redb::WriteTransaction) -> Result<()> {
    let _ = write_txn
        .open_table(REPOS_TABLE)
        .map_err(|e| DomainError::Storage(format!("打开 repos 表失败：{e}")))?;
    Ok(())
}
