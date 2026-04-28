//! macOS 钥匙串集成
//!
//! 在系统钥匙串里存取一个 32 字节的主密钥（Master Key），
//! 主密钥用于加密 redb 里的敏感字段（密码等）。
//!
//! # 安全模型
//!
//! - 钥匙串本身由 macOS 系统加密，登录态解锁
//! - 主密钥永远不写入磁盘（只在钥匙串里）
//! - 即使 redb 文件被人拷走，没有钥匙串里的主密钥也解不出密码
//!
//! # 跨平台
//!
//! `keyring` crate 在 Linux 走 secret-service，Windows 走 Credential Manager，
//! 但 v0.1 优先 macOS 体验，其他平台靠 keyring crate 默认行为兜底。

use rand::TryRngCore;
use ramag_domain::error::{DomainError, Result};
use tracing::{info, warn};

const SERVICE: &str = "ramag";
const ACCOUNT: &str = "master-key";
const KEY_LEN: usize = 32; // AES-256

/// 获取或创建主密钥
///
/// 首次调用：随机生成 32 字节，写入钥匙串
/// 后续调用：从钥匙串直接读出
pub fn get_or_create_master_key() -> Result<[u8; KEY_LEN]> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| DomainError::Storage(format!("初始化钥匙串失败：{e}")))?;

    match entry.get_password() {
        Ok(hex_str) => {
            // 已有主密钥
            let bytes = hex::decode(hex_str.trim())
                .map_err(|e| DomainError::Storage(format!("钥匙串里主密钥格式错误：{e}")))?;
            if bytes.len() != KEY_LEN {
                warn!(
                    actual = bytes.len(),
                    expected = KEY_LEN,
                    "master key length mismatch, regenerating"
                );
                return generate_and_save(&entry);
            }
            let mut key = [0u8; KEY_LEN];
            key.copy_from_slice(&bytes);
            Ok(key)
        }
        Err(keyring::Error::NoEntry) => {
            // 第一次运行，生成
            info!("master key not found, generating new one");
            generate_and_save(&entry)
        }
        Err(e) => Err(DomainError::Storage(format!("读取钥匙串失败：{e}"))),
    }
}

fn generate_and_save(entry: &keyring::Entry) -> Result<[u8; KEY_LEN]> {
    let mut key = [0u8; KEY_LEN];
    rand::rngs::OsRng
        .try_fill_bytes(&mut key)
        .map_err(|e| DomainError::Storage(format!("OS 随机源不可用：{e}")))?;
    entry
        .set_password(&hex::encode(key))
        .map_err(|e| DomainError::Storage(format!("写入钥匙串失败：{e}")))?;
    info!("master key created and stored in keychain");
    Ok(key)
}

/// 测试/调试用：删除钥匙串里的主密钥
///
/// **生产环境慎用**——会导致已加密的所有数据无法解密
#[cfg(any(test, debug_assertions))]
pub fn delete_master_key() -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| DomainError::Storage(format!("初始化钥匙串失败：{e}")))?;
    match entry.delete_credential() {
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(DomainError::Storage(format!("删除钥匙串条目失败：{e}"))),
    }
}
