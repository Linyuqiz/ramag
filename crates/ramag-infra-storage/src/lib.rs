#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

//! 本地存储：redb 嵌入式 DB；密码 AES-GCM 加密，主密钥存 macOS 钥匙串。
//! 业务按表拆到 `repos` 子模块（同步），lib 用 `run_blocking` 异步化。
//! 文件路径（macOS）：`~/Library/Application Support/com.ramag.ramag/ramag.redb`

pub mod encryption;
pub mod keyring;
mod repos;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use directories::ProjectDirs;
use futures::channel::oneshot;
use parking_lot::RwLock;
use redb::Database;
use tracing::info;

use ramag_domain::entities::{
    ConnectionConfig, ConnectionId, QueryRecord, QueryRecordId, RepoConfig, RepoId,
};
use ramag_domain::error::{DomainError, Result};
use ramag_domain::traits::Storage;

use crate::encryption::Cipher;

pub struct RedbStorage {
    db: Arc<Database>,
    cipher: Arc<RwLock<Cipher>>,
    path: PathBuf,
}

impl RedbStorage {
    /// 默认路径，首次会创建文件 + 钥匙串生成主密钥
    pub fn open_default() -> Result<Self> {
        let path = default_db_path()?;
        Self::open(&path)
    }

    /// 生产入口：从系统钥匙串读主密钥
    pub fn open(path: &Path) -> Result<Self> {
        let master_key = keyring::get_or_create_master_key()?;
        Self::open_with_key(path, &master_key)
    }

    /// 测试入口：注入固定密钥避免污染真实钥匙串
    pub fn open_with_key(path: &Path, master_key: &[u8; 32]) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| DomainError::Storage(format!("创建数据目录失败：{e}")))?;
        }

        let db = Database::create(path)
            .map_err(|e| DomainError::Storage(format!("打开 redb 数据库失败：{e}")))?;

        // 首次打开建表
        let write_txn = db
            .begin_write()
            .map_err(|e| DomainError::Storage(format!("启动写事务失败：{e}")))?;
        repos::connection_repo::ensure_table(&write_txn)?;
        repos::repo_repo::ensure_table(&write_txn)?;
        repos::history_repo::ensure_table(&write_txn)?;
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

fn default_db_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("com", "ramag", "ramag")
        .ok_or_else(|| DomainError::Storage("无法定位用户目录".into()))?;
    Ok(dirs.data_dir().join("ramag.redb"))
}

/// std::thread + oneshot 桥接同步代码，调用方任意 runtime 通用
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
        run_blocking(move || repos::connection_repo::list(db, cipher)).await
    }

    async fn get_connection(&self, id: &ConnectionId) -> Result<Option<ConnectionConfig>> {
        let db = self.db.clone();
        let cipher = self.cipher.clone();
        let id_str = id.to_string();
        run_blocking(move || repos::connection_repo::get(db, cipher, id_str)).await
    }

    async fn save_connection(&self, config: &ConnectionConfig) -> Result<()> {
        let db = self.db.clone();
        let cipher = self.cipher.clone();
        let config = config.clone();
        run_blocking(move || repos::connection_repo::save(db, cipher, config)).await
    }

    async fn delete_connection(&self, id: &ConnectionId) -> Result<()> {
        let db = self.db.clone();
        let id_str = id.to_string();
        run_blocking(move || repos::connection_repo::delete(db, id_str)).await
    }

    async fn list_repos(&self) -> Result<Vec<RepoConfig>> {
        let db = self.db.clone();
        run_blocking(move || repos::repo_repo::list(db)).await
    }

    async fn save_repo(&self, config: &RepoConfig) -> Result<()> {
        let db = self.db.clone();
        let config = config.clone();
        run_blocking(move || repos::repo_repo::save(db, config)).await
    }

    async fn delete_repo(&self, id: &RepoId) -> Result<()> {
        let db = self.db.clone();
        let id = id.clone();
        run_blocking(move || repos::repo_repo::delete(db, id)).await
    }

    async fn append_history(&self, record: &QueryRecord) -> Result<()> {
        let db = self.db.clone();
        let record = record.clone();
        run_blocking(move || repos::history_repo::append(db, record)).await
    }

    async fn list_history(
        &self,
        connection_id: Option<&ConnectionId>,
        limit: usize,
    ) -> Result<Vec<QueryRecord>> {
        let db = self.db.clone();
        let conn_filter = connection_id.cloned();
        run_blocking(move || repos::history_repo::list(db, conn_filter, limit)).await
    }

    async fn delete_history(&self, id: &QueryRecordId) -> Result<()> {
        let db = self.db.clone();
        let id = id.clone();
        run_blocking(move || repos::history_repo::delete(db, id)).await
    }

    async fn clear_history(&self, connection_id: Option<&ConnectionId>) -> Result<()> {
        let db = self.db.clone();
        let conn_filter = connection_id.cloned();
        run_blocking(move || repos::history_repo::clear(db, conn_filter)).await
    }

    async fn get_preference(&self, key: &str) -> Result<Option<String>> {
        let db = self.db.clone();
        let key = key.to_string();
        run_blocking(move || repos::prefs_repo::get(db, key)).await
    }

    async fn set_preference(&self, key: &str, value: &str) -> Result<()> {
        let db = self.db.clone();
        let key = key.to_string();
        let value = value.to_string();
        run_blocking(move || repos::prefs_repo::set(db, key, value)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ramag_domain::entities::DriverKind;
    use tempfile::TempDir;

    /// 临时目录 + 固定密钥，不污染真实钥匙串
    fn make_test_storage() -> (RedbStorage, TempDir) {
        let tmp = TempDir::new().expect("创建临时目录失败");
        let path = tmp.path().join("test.redb");
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
        assert_eq!(list[0].password, "secret-password");
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
