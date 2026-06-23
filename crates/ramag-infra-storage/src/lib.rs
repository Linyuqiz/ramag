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
    ClipId, ClipItem, ConnectionConfig, ConnectionId, QueryRecord, QueryRecordId, RepoConfig,
    RepoId,
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
        repos::clip_repo::ensure_table(&write_txn)?;
        write_txn
            .commit()
            .map_err(|e| DomainError::Storage(format!("提交事务失败：{e}")))?;

        let db = Arc::new(db);
        let cipher = Arc::new(RwLock::new(Cipher::new(master_key)));

        // 首启迁移：为存量历史构建时间 / 去重索引（空库或已建则瞬时返回）
        repos::clip_repo::migrate_indexes(db.clone(), cipher.clone())?;

        info!(path = %path.display(), "redb storage opened");

        Ok(Self {
            db,
            cipher,
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

    async fn seal(&self, plain: &[u8]) -> Result<Vec<u8>> {
        let cipher = self.cipher.clone();
        let plain = plain.to_vec();
        run_blocking(move || cipher.read().encrypt_bytes(&plain)).await
    }

    async fn unseal(&self, cipher_blob: &[u8]) -> Result<Vec<u8>> {
        let cipher = self.cipher.clone();
        let blob = cipher_blob.to_vec();
        run_blocking(move || cipher.read().decrypt_bytes(&blob)).await
    }

    async fn clip_save(&self, item: &ClipItem) -> Result<()> {
        let db = self.db.clone();
        let cipher = self.cipher.clone();
        let item = item.clone();
        run_blocking(move || repos::clip_repo::save(db, cipher, item)).await
    }

    async fn clip_list(&self) -> Result<Vec<ClipItem>> {
        let db = self.db.clone();
        let cipher = self.cipher.clone();
        run_blocking(move || repos::clip_repo::list(db, cipher)).await
    }

    async fn clip_list_recent(&self, limit: usize) -> Result<Vec<ClipItem>> {
        let db = self.db.clone();
        let cipher = self.cipher.clone();
        run_blocking(move || repos::clip_repo::list_recent(db, cipher, limit)).await
    }

    async fn clip_search(&self, query: &str, limit: usize) -> Result<Vec<ClipItem>> {
        let db = self.db.clone();
        let cipher = self.cipher.clone();
        let query = query.to_string();
        run_blocking(move || repos::clip_repo::search(db, cipher, query, limit)).await
    }

    async fn clip_delete(&self, id: &ClipId) -> Result<()> {
        let db = self.db.clone();
        let id_str = id.to_string();
        run_blocking(move || repos::clip_repo::delete(db, id_str)).await
    }

    async fn clip_find_by_hash(&self, hash: &str) -> Result<Option<ClipItem>> {
        let db = self.db.clone();
        let cipher = self.cipher.clone();
        let hash = hash.to_string();
        run_blocking(move || repos::clip_repo::find_by_hash(db, cipher, hash)).await
    }

    async fn clip_clear(&self) -> Result<Vec<String>> {
        let db = self.db.clone();
        let cipher = self.cipher.clone();
        run_blocking(move || repos::clip_repo::clear(db, cipher)).await
    }

    async fn clip_prune(&self, max_items: u32, max_age_days: u32) -> Result<Vec<String>> {
        let db = self.db.clone();
        let cipher = self.cipher.clone();
        run_blocking(move || repos::clip_repo::prune(db, cipher, max_items, max_age_days)).await
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
            auth_source: None,
            remark: None,
            production: false,
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

    use chrono::{Duration, Utc};
    use ramag_domain::entities::{ClipId, ClipKind};

    fn sample_clip(text: &str, age_days: i64) -> ramag_domain::entities::ClipItem {
        let at = Utc::now() - Duration::days(age_days);
        ramag_domain::entities::ClipItem {
            id: ClipId::new(),
            kind: ClipKind::Text,
            text: Some(text.to_string()),
            rtf: None,
            image_path: None,
            thumb_path: None,
            image_dims: None,
            files: Vec::new(),
            preview: text.to_string(),
            source: None,
            byte_size: text.len() as u64,
            content_hash: format!(
                "{:016x}",
                ramag_domain::entities::fnv1a_hash(text.as_bytes())
            ),
            created_at: at,
            last_used_at: at,
        }
    }

    #[tokio::test]
    async fn clip_save_list_roundtrip_sorted() {
        let (storage, _tmp) = make_test_storage();
        storage.clip_save(&sample_clip("old", 3)).await.unwrap();
        storage.clip_save(&sample_clip("new", 0)).await.unwrap();

        let list = storage.clip_list().await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].text.as_deref(), Some("new"));
        assert_eq!(list[1].text.as_deref(), Some("old"));
    }

    #[tokio::test]
    async fn clip_find_by_hash_and_delete() {
        let (storage, _tmp) = make_test_storage();
        let clip = sample_clip("dup-me", 0);
        storage.clip_save(&clip).await.unwrap();

        let found = storage.clip_find_by_hash(&clip.content_hash).await.unwrap();
        assert_eq!(found.unwrap().id, clip.id);
        assert!(storage.clip_find_by_hash("ffff").await.unwrap().is_none());

        storage.clip_delete(&clip.id).await.unwrap();
        assert!(storage.clip_list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn clip_clear_removes_all() {
        let (storage, _tmp) = make_test_storage();
        storage.clip_save(&sample_clip("a", 0)).await.unwrap();
        storage.clip_save(&sample_clip("b", 0)).await.unwrap();

        storage.clip_clear().await.unwrap();
        assert!(storage.clip_list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn clip_prune_by_count_and_age() {
        let (storage, _tmp) = make_test_storage();
        storage
            .clip_save(&sample_clip("expired", 40))
            .await
            .unwrap();
        storage.clip_save(&sample_clip("kept-1", 1)).await.unwrap();
        storage.clip_save(&sample_clip("kept-2", 0)).await.unwrap();

        // 数量上限 5、保留 30 天：仅超龄 expired 被剔
        storage.clip_prune(5, 30).await.unwrap();
        let rest = storage.clip_list().await.unwrap();
        let texts: Vec<_> = rest.iter().map(|c| c.text.clone().unwrap()).collect();
        assert_eq!(rest.len(), 2);
        assert!(texts.contains(&"kept-1".to_string()));
        assert!(texts.contains(&"kept-2".to_string()));

        // 数量上限 1：只留最新
        storage.clip_prune(1, 30).await.unwrap();
        let rest = storage.clip_list().await.unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].text.as_deref(), Some("kept-2"));
    }

    #[tokio::test]
    async fn clip_list_recent_order_and_limit() {
        let (storage, _tmp) = make_test_storage();
        storage.clip_save(&sample_clip("oldest", 3)).await.unwrap();
        storage.clip_save(&sample_clip("mid", 2)).await.unwrap();
        storage.clip_save(&sample_clip("newest", 0)).await.unwrap();

        // limit 截断 + 最近优先
        let recent = storage.clip_list_recent(2).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].text.as_deref(), Some("newest"));
        assert_eq!(recent[1].text.as_deref(), Some("mid"));

        // limit 超总数 → 全部返回
        let all = storage.clip_list_recent(100).await.unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn clip_update_refreshes_recency_without_dup() {
        let (storage, _tmp) = make_test_storage();
        let mut a = sample_clip("a", 5);
        let b = sample_clip("b", 0);
        storage.clip_save(&a).await.unwrap();
        storage.clip_save(&b).await.unwrap();
        assert_eq!(
            storage.clip_list_recent(10).await.unwrap()[0]
                .text
                .as_deref(),
            Some("b")
        );

        // 提升 a（同 id 更新 last_used）→ 旧时间索引项须清除，不得产生重复
        a.last_used_at = Utc::now();
        storage.clip_save(&a).await.unwrap();
        let r = storage.clip_list_recent(10).await.unwrap();
        assert_eq!(r.len(), 2, "更新不应产生重复条目");
        assert_eq!(r[0].text.as_deref(), Some("a"));
        assert_eq!(r[1].text.as_deref(), Some("b"));
    }

    #[tokio::test]
    async fn clip_migrate_rebuilds_indexes_from_main_table() {
        let (storage, _tmp) = make_test_storage();
        let c1 = sample_clip("alpha", 2);
        let c2 = sample_clip("beta", 0);
        storage.clip_save(&c1).await.unwrap();
        storage.clip_save(&c2).await.unwrap();

        // 模拟索引丢失：删时间索引表后重建空表（主表保留），mirror open 时 ensure→migrate 流程
        {
            let txn = storage.db.begin_write().unwrap();
            txn.delete_table(repos::clip_repo::CLIP_BY_TIME).unwrap();
            repos::clip_repo::ensure_table(&txn).unwrap();
            txn.commit().unwrap();
        }
        repos::clip_repo::migrate_indexes(storage.db.clone(), storage.cipher.clone()).unwrap();

        let recent = storage.clip_list_recent(10).await.unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].text.as_deref(), Some("beta"));
        assert!(
            storage
                .clip_find_by_hash(&c1.content_hash)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn clip_search_matches_recent_first_and_limit() {
        let (storage, _tmp) = make_test_storage();
        storage
            .clip_save(&sample_clip("hello world", 2))
            .await
            .unwrap();
        storage.clip_save(&sample_clip("foo bar", 1)).await.unwrap();
        storage
            .clip_save(&sample_clip("hello rust", 0))
            .await
            .unwrap();

        // 匹配 + 最近优先
        let r = storage.clip_search("hello", 10).await.unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].text.as_deref(), Some("hello rust"));
        assert_eq!(r[1].text.as_deref(), Some("hello world"));

        // limit 早停
        let r = storage.clip_search("hello", 1).await.unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text.as_deref(), Some("hello rust"));

        // 空 query / 无匹配 → 空
        assert!(storage.clip_search("", 10).await.unwrap().is_empty());
        assert!(storage.clip_search("zzz", 10).await.unwrap().is_empty());
    }
}
