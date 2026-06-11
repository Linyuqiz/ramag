//! AES-256-GCM 加密。格式：`nonce(12) || ciphertext || tag(16)`，hex 编码后落库

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use ramag_domain::error::{DomainError, Result};

pub struct Cipher {
    inner: Aes256Gcm,
}

impl Cipher {
    pub fn new(master_key: &[u8; 32]) -> Self {
        let key = Key::<Aes256Gcm>::from_slice(master_key);
        Self {
            inner: Aes256Gcm::new(key),
        }
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .inner
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|e| DomainError::Storage(format!("加密失败：{e}")))?;

        let mut blob = Vec::with_capacity(12 + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);
        Ok(hex::encode(blob))
    }

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

    /// 加密任意字节，返回 `nonce(12) || ciphertext || tag(16)` 原始字节（不 hex，图片落盘用）
    pub fn encrypt_bytes(&self, plain: &[u8]) -> Result<Vec<u8>> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .inner
            .encrypt(&nonce, plain)
            .map_err(|e| DomainError::Storage(format!("加密失败：{e}")))?;
        let mut blob = Vec::with_capacity(12 + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);
        Ok(blob)
    }

    /// 解密 `encrypt_bytes` 产物
    pub fn decrypt_bytes(&self, blob: &[u8]) -> Result<Vec<u8>> {
        if blob.len() < 12 + 16 {
            return Err(DomainError::Storage("密文长度异常".into()));
        }
        let (nonce_bytes, ciphertext) = blob.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        self.inner
            .decrypt(nonce, ciphertext)
            .map_err(|e| DomainError::Storage(format!("解密失败（密钥可能错误）：{e}")))
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
        // hex::decode 大小写不敏感，必须换到值不同的字符
        let bytes = encrypted.as_bytes();
        let idx = bytes.len() - 2;
        let replacement: u8 = if bytes[idx] == b'0' { b'1' } else { b'0' };
        let mut tampered = encrypted.clone();
        unsafe {
            tampered.as_bytes_mut()[idx] = replacement;
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
    fn bytes_round_trip() {
        let cipher = Cipher::new(&dummy_key());
        let data = vec![0u8, 1, 2, 255, 128, 13, 10, 0];
        let enc = cipher.encrypt_bytes(&data).unwrap();
        assert_ne!(enc, data);
        assert_eq!(cipher.decrypt_bytes(&enc).unwrap(), data);
        // 篡改任意字节 → 解密失败
        let mut bad = enc.clone();
        let last = bad.len() - 1;
        bad[last] ^= 0xff;
        assert!(cipher.decrypt_bytes(&bad).is_err());
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
