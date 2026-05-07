//! Redis key 空间元数据：SCAN 浏览的 (name, type, ttl) 载体

use serde::{Deserialize, Serialize};

/// 与 `TYPE <key>` 应答对齐
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RedisType {
    String,
    List,
    Hash,
    Set,
    ZSet,
    Stream,
    /// key 不存在
    None,
}

impl RedisType {
    /// `TYPE` 应答 → 枚举。未知（模块自定义）映射为 None
    pub fn parse(s: &str) -> Self {
        match s {
            "string" => RedisType::String,
            "list" => RedisType::List,
            "hash" => RedisType::Hash,
            "set" => RedisType::Set,
            "zset" => RedisType::ZSet,
            "stream" => RedisType::Stream,
            _ => RedisType::None,
        }
    }

    /// `SCAN ... TYPE <type>` 用的小写字面量
    pub fn as_scan_arg(&self) -> &'static str {
        match self {
            RedisType::String => "string",
            RedisType::List => "list",
            RedisType::Hash => "hash",
            RedisType::Set => "set",
            RedisType::ZSet => "zset",
            RedisType::Stream => "stream",
            RedisType::None => "none",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            RedisType::String => "String",
            RedisType::List => "List",
            RedisType::Hash => "Hash",
            RedisType::Set => "Set",
            RedisType::ZSet => "ZSet",
            RedisType::Stream => "Stream",
            RedisType::None => "(none)",
        }
    }
}

/// Key 元数据。SCAN 阶段 `key_type` / `ttl_ms` 可为 None，UI 按需补查
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyMeta {
    /// utf-8 字符串，driver 保证（暂不支持二进制 key）
    pub key: String,
    /// None=未查询，Some(RedisType::None)=查过但 key 不存在
    pub key_type: Option<RedisType>,
    /// PTTL：None=未查询，-1=永久，-2=key 不存在，>=0=剩余毫秒
    pub ttl_ms: Option<i64>,
}

impl KeyMeta {
    /// 仅 key 名的最简元数据
    pub fn bare(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            key_type: None,
            ttl_ms: None,
        }
    }
}

/// SCAN 一批应答
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    /// 下次游标，0 = 遍历结束
    pub cursor: u64,
    pub keys: Vec<KeyMeta>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_types() {
        assert_eq!(RedisType::parse("string"), RedisType::String);
        assert_eq!(RedisType::parse("zset"), RedisType::ZSet);
        assert_eq!(RedisType::parse("stream"), RedisType::Stream);
    }

    #[test]
    fn parse_unknown_falls_back_to_none() {
        assert_eq!(RedisType::parse("module"), RedisType::None);
        assert_eq!(RedisType::parse(""), RedisType::None);
    }

    #[test]
    fn scan_arg_roundtrip() {
        for t in [
            RedisType::String,
            RedisType::List,
            RedisType::Hash,
            RedisType::Set,
            RedisType::ZSet,
            RedisType::Stream,
        ] {
            assert_eq!(RedisType::parse(t.as_scan_arg()), t);
        }
    }
}
