//! aes-gcm 加密层
//!
//! 用主密钥（来自钥匙串）加密敏感字段。
//!
//! # 格式
//!
//! 加密后 = `nonce(12 字节) || ciphertext || tag(16 字节内嵌于 aes-gcm 输出)`
//! 序列化为 hex 字符串方便 JSON 存储

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use ramag_domain::error::{DomainError, Result};

/// AES-256-GCM 加密器
pub struct Cipher {
    inner: Aes256Gcm,
}

impl Cipher {
    /// 用 32 字节主密钥构造
    pub fn new(master_key: &[u8; 32]) -> Self {
        let key = Key::<Aes256Gcm>::from_slice(master_key);
        Self {
            inner: Aes256Gcm::new(key),
        }
    }

    /// 加密明文，返回 hex 字符串
    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .inner
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|e| DomainError::Storage(format!("加密失败：{e}")))?;

        // nonce(12) || ciphertext+tag
        let mut blob = Vec::with_capacity(12 + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);
        Ok(hex::encode(blob))
    }

    /// 解密 hex 字符串，返回明文
    pub fn decrypt(&self, hex_blob: &str) -> Result<String> {
        let blob = hex::decode(hex_blob)
            .map_err(|e| DomainError::Storage(format!("密文 hex 解析失败：{e}")))?;

        if blob.len() < 12 + 16 {
            return Err(DomainError::Storage("密文长度异常".into()));
        }

        let (nonce_bytes, ciphertext) = blob.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = self
            .inner
            .decrypt(nonce, ciphertext)
            .map_err(|e| DomainError::Storage(format!("解密失败（密钥可能错误）：{e}")))?;

        String::from_utf8(plaintext)
            .map_err(|e| DomainError::Storage(format!("解密结果不是 UTF-8：{e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_key() -> [u8; 32] {
        let mut k = [0u8; 32];
        for (i, b) in k.iter_mut().enumerate() {
            *b = i as u8;
        }
        k
    }

    #[test]
    fn round_trip() {
        let cipher = Cipher::new(&dummy_key());
        let original = "Midas@Mysql2027!";
        let encrypted = cipher.encrypt(original).unwrap();
        let decrypted = cipher.decrypt(&encrypted).unwrap();
        assert_eq!(original, decrypted);
    }

    #[test]
    fn encrypt_produces_different_ciphertext_each_time() {
        // 同一明文每次加密 nonce 不同，密文不同
        let cipher = Cipher::new(&dummy_key());
        let c1 = cipher.encrypt("abc").unwrap();
        let c2 = cipher.encrypt("abc").unwrap();
        assert_ne!(c1, c2);
    }

    #[test]
    fn wrong_key_fails() {
        let key1 = dummy_key();
        let mut key2 = key1;
        key2[0] ^= 0xff;

        let c1 = Cipher::new(&key1);
        let c2 = Cipher::new(&key2);
        let encrypted = c1.encrypt("secret").unwrap();
        assert!(c2.decrypt(&encrypted).is_err());
    }

    #[test]
    fn corrupted_ciphertext_fails() {
        let cipher = Cipher::new(&dummy_key());
        let encrypted = cipher.encrypt("hello").unwrap();
        // 篡改：取倒数第二字节，与一个保证不同的 base64 字符替换
        // 之前用 .pop().push('0') 在末尾刚好是 '0' 时无效，flaky
        let bytes = encrypted.as_bytes();
        let idx = bytes.len() - 2;
        let original = bytes[idx] as char;
        let replacement = if original == 'A' { 'B' } else { 'A' };
        let mut tampered = encrypted.clone();
        unsafe {
            tampered.as_bytes_mut()[idx] = replacement as u8;
        }
        assert_ne!(tampered, encrypted);
        assert!(cipher.decrypt(&tampered).is_err());
    }

    #[test]
    fn empty_plaintext() {
        let cipher = Cipher::new(&dummy_key());
        let encrypted = cipher.encrypt("").unwrap();
        let decrypted = cipher.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn unicode_plaintext() {
        let cipher = Cipher::new(&dummy_key());
        let original = "中文密码🔐";
        let encrypted = cipher.encrypt(original).unwrap();
        let decrypted = cipher.decrypt(&encrypted).unwrap();
        assert_eq!(original, decrypted);
    }
}
