//! Redis 运行时值。不实现 Eq/Hash（内部含 f64 ZSet score）

use serde::{Deserialize, Serialize};

/// Redis 单 key 完整值。`KvDriver::get_value` 按 TYPE 自动 dispatch
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RedisValue {
    /// key 不存在 / nil bulk
    Nil,
    /// UTF-8 可解码 String
    Text(String),
    /// UTF-8 解码失败的 fallback，或 BLOB
    Bytes(Vec<u8>),
    /// INCR 应答 / String 数字编码
    Int(i64),
    /// RESP3 Double / ZSCORE
    Float(f64),
    /// RESP3 Boolean
    Bool(bool),
    /// 保留服务端顺序
    List(Vec<RedisValue>),
    /// 用 Vec 保留 HSET 顺序
    Hash(Vec<(String, RedisValue)>),
    /// 唯一元素由服务端保证
    Set(Vec<RedisValue>),
    /// (member, score)，按 score 升序
    ZSet(Vec<(RedisValue, f64)>),
    /// 按时间序排列
    Stream(Vec<StreamEntry>),
    /// 通用数组（CONFIG GET / CLUSTER NODES 等复合应答）
    Array(Vec<RedisValue>),
}

/// Stream 单条消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamEntry {
    /// 形如 `<ms>-<seq>`
    pub id: String,
    /// XADD 的 key=value 列表
    pub fields: Vec<(String, String)>,
}

impl RedisValue {
    pub fn is_nil(&self) -> bool {
        matches!(self, RedisValue::Nil)
    }

    /// 元素数量；标量返回 None
    pub fn len(&self) -> Option<usize> {
        match self {
            RedisValue::List(v) | RedisValue::Set(v) | RedisValue::Array(v) => Some(v.len()),
            RedisValue::Hash(v) => Some(v.len()),
            RedisValue::ZSet(v) => Some(v.len()),
            RedisValue::Stream(v) => Some(v.len()),
            _ => None,
        }
    }

    /// 标量固定返回 false
    pub fn is_empty(&self) -> bool {
        self.len().is_some_and(|n| n == 0)
    }

    /// UI 单行预览，截断长字符串
    pub fn display_preview(&self, max_len: usize) -> String {
        match self {
            RedisValue::Nil => "(nil)".to_string(),
            RedisValue::Text(s) => sanitize_inline(&truncate(s, max_len)),
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

/// 单行预览清洗：换行符（\n / \r）替换为空格。
/// GPUI 单行文本 shaping 断言不允许 \n（含 \n 直接 panic→abort）；仅用于显示预览。
/// 无换行时零拷贝
fn sanitize_inline(s: &str) -> String {
    if s.contains(['\n', '\r']) {
        s.replace(['\n', '\r'], " ")
    } else {
        s.to_string()
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

    #[test]
    fn preview_text_strips_newlines() {
        // 含换行的 String 值预览必须压成单行，否则 key 详情渲染 panic
        let v = RedisValue::Text("line1\nline2\r\nline3".to_string());
        let p = v.display_preview(80);
        assert!(!p.contains('\n') && !p.contains('\r'));
    }
}
