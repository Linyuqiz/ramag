//! macOS 钥匙串：存 32 字节主密钥（Master Key），用于加密 redb 敏感字段。
//! 主密钥仅存钥匙串，不落盘；redb 文件被拷走也解不出密码。
//! 跨平台：keyring crate 在 Linux/Windows 走对应原生方案

use ramag_domain::error::{DomainError, Result};
use rand::TryRngCore;
use tracing::{info, warn};

const SERVICE: &str = "ramag";
const ACCOUNT: &str = "master-key";
const KEY_LEN: usize = 32;

/// 首次随机生成 + 写钥匙串；后续读出
pub fn get_or_create_master_key() -> Result<[u8; KEY_LEN]> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| DomainError::Storage(format!("初始化钥匙串失败：{e}")))?;

    match entry.get_password() {
        Ok(hex_str) => {
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

/// 测试 / 调试用，生产慎用：删除会让已加密数据全部无法解密
#[cfg(any(test, debug_assertions))]
pub fn delete_master_key() -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| DomainError::Storage(format!("初始化钥匙串失败：{e}")))?;
    match entry.delete_credential() {
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(DomainError::Storage(format!("删除钥匙串条目失败：{e}"))),
    }
}
