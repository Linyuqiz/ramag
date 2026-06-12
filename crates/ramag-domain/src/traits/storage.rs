//! Storage trait：本地持久化统一抽象。infra 层用 redb 实现

use async_trait::async_trait;

use crate::entities::{
    ClipId, ClipItem, ConnectionConfig, ConnectionId, QueryRecord, QueryRecordId, RepoConfig,
    RepoId,
};
use crate::error::Result;

#[async_trait]
pub trait Storage: Send + Sync {
    // 连接配置
    async fn list_connections(&self) -> Result<Vec<ConnectionConfig>>;
    async fn get_connection(&self, id: &ConnectionId) -> Result<Option<ConnectionConfig>>;
    /// 新增或更新
    async fn save_connection(&self, config: &ConnectionConfig) -> Result<()>;
    async fn delete_connection(&self, id: &ConnectionId) -> Result<()>;

    // Git 仓库（VCS 最近仓库列表）

    /// 按 name 字母序，列表顺序稳定不随打开顺序漂移
    async fn list_repos(&self) -> Result<Vec<RepoConfig>> {
        Err(crate::error::DomainError::NotImplemented(
            "list_repos".into(),
        ))
    }

    /// 新增或更新。VCS 打开仓库后会先更新 `last_opened_at` 再调
    async fn save_repo(&self, _config: &RepoConfig) -> Result<()> {
        Err(crate::error::DomainError::NotImplemented(
            "save_repo".into(),
        ))
    }

    /// 仅从最近列表移除，不影响磁盘文件
    async fn delete_repo(&self, _id: &RepoId) -> Result<()> {
        Err(crate::error::DomainError::NotImplemented(
            "delete_repo".into(),
        ))
    }

    // 查询历史

    async fn append_history(&self, record: &QueryRecord) -> Result<()>;

    /// 按 executed_at desc。connection_id=None 全部连接
    async fn list_history(
        &self,
        connection_id: Option<&ConnectionId>,
        limit: usize,
    ) -> Result<Vec<QueryRecord>>;

    async fn delete_history(&self, id: &QueryRecordId) -> Result<()>;

    /// connection_id=None 清空全部
    async fn clear_history(&self, connection_id: Option<&ConnectionId>) -> Result<()>;

    // 偏好 KV
    async fn get_preference(&self, key: &str) -> Result<Option<String>>;
    async fn set_preference(&self, key: &str, value: &str) -> Result<()>;

    /// 用主密钥 AES-GCM 加密任意字节（剪贴图片落盘前调，密文存磁盘）
    async fn seal(&self, _plain: &[u8]) -> Result<Vec<u8>> {
        Err(crate::error::DomainError::NotImplemented("seal".into()))
    }

    /// 解密 `seal` 产物
    async fn unseal(&self, _cipher: &[u8]) -> Result<Vec<u8>> {
        Err(crate::error::DomainError::NotImplemented("unseal".into()))
    }

    // 剪贴板历史（默认 NotImplemented，与 repos 同策略：旧 mock 实现不强制跟进）

    /// 新增或更新（按 id upsert）
    async fn clip_save(&self, _item: &ClipItem) -> Result<()> {
        Err(crate::error::DomainError::NotImplemented(
            "clip_save".into(),
        ))
    }

    /// 按 last_used_at desc 返回全部
    async fn clip_list(&self) -> Result<Vec<ClipItem>> {
        Err(crate::error::DomainError::NotImplemented(
            "clip_list".into(),
        ))
    }

    async fn clip_delete(&self, _id: &ClipId) -> Result<()> {
        Err(crate::error::DomainError::NotImplemented(
            "clip_delete".into(),
        ))
    }

    /// 内容指纹查重（连续复制同内容时提升旧条目）
    async fn clip_find_by_hash(&self, _hash: &str) -> Result<Option<ClipItem>> {
        Err(crate::error::DomainError::NotImplemented(
            "clip_find_by_hash".into(),
        ))
    }

    /// 清空全部历史。返回被删条目的 image_path（调用方清理落盘文件）
    async fn clip_clear(&self) -> Result<Vec<String>> {
        Err(crate::error::DomainError::NotImplemented(
            "clip_clear".into(),
        ))
    }

    /// 超量 / 过期清理。返回被删条目的 image_path
    async fn clip_prune(&self, _max_items: u32, _max_age_days: u32) -> Result<Vec<String>> {
        Err(crate::error::DomainError::NotImplemented(
            "clip_prune".into(),
        ))
    }
}
