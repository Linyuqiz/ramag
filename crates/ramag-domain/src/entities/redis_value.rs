//! Redis 值类型实体
//!
//! 用统一的 enum 表达 Redis 各类数据结构的"运行时值"：
//! - 标量（String/Int/Bytes）
//! - 复合（List/Hash/Set/ZSet/Stream）
//! - 嵌套数组（命令应答的通用形态）
//!
//! 与 [`crate::entities::query::Value`] 的关系：
//! - SQL 类驱动（MySQL/PG）使用 `Value`（按列展开成行）
//! - KV 类驱动（Redis）使用 `RedisValue`（按 key 形态封装）
//!
//! 不实现 Eq/Hash：内部含 f64（ZSet score），不满足全序

use serde::{Deserialize, Serialize};

/// Redis 单 key 完整值
///
/// 通过 [`crate::traits::KvDriver::get_value`] 返回，按 key 类型自动 dispatch
/// （HGETALL / LRANGE 0 -1 / ZRANGE WITHSCORES / XRANGE - + 等）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RedisValue {
    /// key 不存在 / nil bulk
    Nil,

    /// String 值（UTF-8 可解码时使用此变体）
    Text(String),

    /// 二进制字节（UTF-8 解码失败 fallback；BLOB 缓存常见）
    Bytes(Vec<u8>),

    /// 整数（INCR 等命令的应答；Redis 内部 String 数字编码）
    Int(i64),

    /// 浮点数（RESP3 Double / ZSCORE 等）
    Float(f64),

    /// 布尔（RESP3 Boolean）
    Bool(bool),

    /// List：有序元素（保留服务端顺序）
    List(Vec<RedisValue>),

    /// Hash：field → value 映射；用 Vec 保留 HSET 顺序
    Hash(Vec<(String, RedisValue)>),

    /// Set：无序唯一元素（客户端不强制去重，由服务端保证）
    Set(Vec<RedisValue>),

    /// Sorted Set：(member, score) 对，按服务端 score 升序
    ZSet(Vec<(RedisValue, f64)>),

    /// Stream：消息条目按时间序排列
    Stream(Vec<StreamEntry>),

    /// 通用数组（命令应答的复合返回，例如 CONFIG GET / CLUSTER NODES）
    Array(Vec<RedisValue>),
}

/// Stream 单条消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEntry {
    /// 条目 ID，格式 `<ms>-<seq>`，如 `1714291200000-0`
    pub id: String,
    /// 字段对（XADD 时的 key=value 列表）
    pub fields: Vec<(String, String)>,
}

impl RedisValue {
    /// 是否为 Nil（key 不存在）
    pub fn is_nil(&self) -> bool {
        matches!(self, RedisValue::Nil)
    }

    /// 元素数量（仅复合类型，标量返回 None）
    ///
    /// UI 在 key 列表显示 size 时调用
    pub fn len(&self) -> Option<usize> {
        match self {
            RedisValue::List(v) | RedisValue::Set(v) | RedisValue::Array(v) => Some(v.len()),
            RedisValue::Hash(v) => Some(v.len()),
            RedisValue::ZSet(v) => Some(v.len()),
            RedisValue::Stream(v) => Some(v.len()),
            _ => None,
        }
    }

    /// 是否为空容器；标量类型固定返回 false
    pub fn is_empty(&self) -> bool {
        self.len().is_some_and(|n| n == 0)
    }

    /// 适合 UI 单行预览的简短文本（截断长字符串）
    pub fn display_preview(&self, max_len: usize) -> String {
        match self {
            RedisValue::Nil => "(nil)".to_string(),
            RedisValue::Text(s) => truncate(s, max_len),
            RedisValue::Bytes(b) => format!("[{} bytes]", b.len()),
            RedisValue::Int(i) => i.to_string(),
            RedisValue::Float(f) => f.to_string(),
            RedisValue::Bool(b) => b.to_string(),
            RedisValue::List(v) => format!("List({} elems)", v.len()),
            RedisValue::Hash(v) => format!("Hash({} fields)", v.len()),
            RedisValue::Set(v) => format!("Set({} elems)", v.len()),
            RedisValue::ZSet(v) => format!("ZSet({} elems)", v.len()),
            RedisValue::Stream(v) => format!("Stream({} entries)", v.len()),
            RedisValue::Array(v) => format!("Array({} elems)", v.len()),
        }
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len).collect();
        format!("{truncated}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nil_is_nil() {
        assert!(RedisValue::Nil.is_nil());
        assert!(!RedisValue::Int(0).is_nil());
    }

    #[test]
    fn len_for_composites() {
        assert_eq!(RedisValue::Text("a".into()).len(), None);
        assert_eq!(
            RedisValue::List(vec![RedisValue::Int(1), RedisValue::Int(2)]).len(),
            Some(2)
        );
        assert_eq!(
            RedisValue::Hash(vec![("k".into(), RedisValue::Int(1))]).len(),
            Some(1)
        );
    }

    #[test]
    fn preview_truncates_long_text() {
        let long: String = "a".repeat(100);
        let preview = RedisValue::Text(long).display_preview(10);
        assert!(preview.ends_with('…'));
        assert!(preview.chars().count() <= 11);
    }

    #[test]
    fn preview_bytes_shows_size() {
        let v = RedisValue::Bytes(vec![0u8; 1024]);
        assert_eq!(v.display_preview(80), "[1024 bytes]");
    }
}
