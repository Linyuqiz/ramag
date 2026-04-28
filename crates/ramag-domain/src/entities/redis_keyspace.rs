//! Redis Key 空间元数据
//!
//! 用于 SCAN 浏览：每个 key 携带 `(name, type, ttl, size)` 用于 UI 表/树展示

use serde::{Deserialize, Serialize};

/// Key 类型（与 Redis `TYPE <key>` 应答对齐）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RedisType {
    String,
    List,
    Hash,
    Set,
    ZSet,
    Stream,
    /// `TYPE` 返回 `none` —— key 不存在
    None,
}

impl RedisType {
    /// Redis TYPE 应答字符串 → 枚举
    ///
    /// 未知类型（如模块自定义类型）映射为 None；上层若需保留原值，请改走原始命令
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

    /// 给 SCAN ... TYPE <type> 用的字面量（小写，与服务端一致）
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

    /// UI 显示用的人类可读标签
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

/// Key 元数据
///
/// 由 SCAN + 后续 TYPE/PTTL 组合得到。`key_type` 和 `ttl_ms` 在 SCAN 阶段
/// 可能为 None（未取），UI 层按需补查
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyMeta {
    /// Key 名（utf-8 字符串；二进制 key 暂不支持，由 driver 保证 utf-8）
    pub key: String,
    /// 类型（None = 未查询；Some(RedisType::None) = 查过但 key 不存在）
    pub key_type: Option<RedisType>,
    /// 剩余 TTL（毫秒）
    /// - None: 未查询
    /// - Some(-1): 永久（无 TTL）
    /// - Some(-2): key 不存在
    /// - Some(n>=0): 剩余毫秒
    pub ttl_ms: Option<i64>,
}

impl KeyMeta {
    /// 仅有 key 名的最简元数据
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
    /// 下次迭代游标；0 表示遍历结束
    pub cursor: u64,
    /// 本批 key 列表
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
