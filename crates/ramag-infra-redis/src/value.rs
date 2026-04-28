//! redis::Value (RESP) → ramag_domain::RedisValue 转换
//!
//! redis-rs 把 RESP 应答统一表达为 `redis::Value`：
//! - RESP2: Nil / Int / BulkString / Array / SimpleString / Okay
//! - RESP3: + Map / Set / Double / Boolean / VerbatimString / BigNumber / Push
//!
//! 本模块负责把这些应答原样翻译到 Domain 的 RedisValue；
//! 类型专属命令（HGETALL/LRANGE/ZRANGE/XRANGE）由 driver 调用配套
//! 解码函数（[`decode_hash_pairs`] 等）整理结构

use ramag_domain::entities::{RedisValue, StreamEntry};
use ramag_domain::error::{DomainError, Result};
use redis::Value as RV;

/// 通用 RESP → RedisValue
///
/// 对未识别 / 不应在此层出现的变体（Push / Attribute / ServerError）兜底返回 Array
pub fn decode_value(v: RV) -> RedisValue {
    match v {
        RV::Nil => RedisValue::Nil,
        RV::Int(i) => RedisValue::Int(i),
        RV::BulkString(bytes) => decode_bulk(bytes),
        RV::Array(arr) => RedisValue::Array(arr.into_iter().map(decode_value).collect()),
        RV::SimpleString(s) => RedisValue::Text(s),
        RV::Okay => RedisValue::Text("OK".into()),
        RV::Map(pairs) => decode_map(pairs),
        RV::Set(items) => RedisValue::Set(items.into_iter().map(decode_value).collect()),
        RV::Double(f) => RedisValue::Float(f),
        RV::Boolean(b) => RedisValue::Bool(b),
        RV::VerbatimString { text, .. } => RedisValue::Text(text),
        RV::BigNumber(s) => RedisValue::Text(s.to_string()),
        RV::Push { data, .. } => RedisValue::Array(data.into_iter().map(decode_value).collect()),
        // ServerError / Attribute 兜底：转成简短文本，不当 Err 抛
        // （正常路径上服务端错误已在 redis-rs 内部包装为 RedisError，到不了此处）
        other => RedisValue::Text(format!("{other:?}")),
    }
}

/// BulkString → Text（UTF-8 成功时）/ Bytes（失败时 fallback）
fn decode_bulk(bytes: Vec<u8>) -> RedisValue {
    match String::from_utf8(bytes) {
        Ok(s) => RedisValue::Text(s),
        Err(e) => RedisValue::Bytes(e.into_bytes()),
    }
}

/// RESP3 Map → RedisValue::Hash（key 强制 utf-8 字符串；非 utf-8 则 fallback Array）
fn decode_map(pairs: Vec<(RV, RV)>) -> RedisValue {
    let mut hash: Vec<(String, RedisValue)> = Vec::with_capacity(pairs.len());
    for (k, v) in pairs {
        match k {
            RV::SimpleString(s) => hash.push((s, decode_value(v))),
            RV::BulkString(bytes) => match String::from_utf8(bytes) {
                Ok(s) => hash.push((s, decode_value(v))),
                Err(_) => return fallback_map_to_array(hash, v),
            },
            other => return fallback_map_to_array_other(hash, other, v),
        }
    }
    RedisValue::Hash(hash)
}

fn fallback_map_to_array(partial: Vec<(String, RedisValue)>, last_v: RV) -> RedisValue {
    let mut arr: Vec<RedisValue> = partial
        .into_iter()
        .flat_map(|(k, v)| [RedisValue::Text(k), v])
        .collect();
    arr.push(decode_value(last_v));
    RedisValue::Array(arr)
}

fn fallback_map_to_array_other(
    partial: Vec<(String, RedisValue)>,
    other_k: RV,
    last_v: RV,
) -> RedisValue {
    let mut arr: Vec<RedisValue> = partial
        .into_iter()
        .flat_map(|(k, v)| [RedisValue::Text(k), v])
        .collect();
    arr.push(decode_value(other_k));
    arr.push(decode_value(last_v));
    RedisValue::Array(arr)
}

/// HGETALL 应答（扁平 [field, value, field, value, ...]）→ RedisValue::Hash
pub fn decode_hash_pairs(v: RV) -> Result<RedisValue> {
    let arr = match v {
        RV::Array(a) => a,
        // RESP3 直接返回 Map
        RV::Map(pairs) => return Ok(decode_map(pairs)),
        RV::Nil => return Ok(RedisValue::Nil),
        other => {
            return Err(DomainError::QueryFailed(format!(
                "HGETALL 应答非数组：{other:?}"
            )));
        }
    };

    if arr.len() % 2 != 0 {
        return Err(DomainError::QueryFailed(format!(
            "HGETALL 应答长度非偶数：{}",
            arr.len()
        )));
    }

    let mut pairs: Vec<(String, RedisValue)> = Vec::with_capacity(arr.len() / 2);
    let mut iter = arr.into_iter();
    while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
        let key = match decode_value(k) {
            RedisValue::Text(s) => s,
            RedisValue::Bytes(b) => String::from_utf8_lossy(&b).into_owned(),
            other => other.display_preview(64),
        };
        pairs.push((key, decode_value(v)));
    }
    Ok(RedisValue::Hash(pairs))
}

/// ZRANGE WITHSCORES 应答（[member, score, member, score, ...]）→ RedisValue::ZSet
pub fn decode_zset_with_scores(v: RV) -> Result<RedisValue> {
    let arr = match v {
        RV::Array(a) => a,
        RV::Nil => return Ok(RedisValue::Nil),
        other => {
            return Err(DomainError::QueryFailed(format!(
                "ZRANGE WITHSCORES 应答非数组：{other:?}"
            )));
        }
    };

    if arr.len() % 2 != 0 {
        return Err(DomainError::QueryFailed(format!(
            "ZRANGE WITHSCORES 应答长度非偶数：{}",
            arr.len()
        )));
    }

    let mut pairs: Vec<(RedisValue, f64)> = Vec::with_capacity(arr.len() / 2);
    let mut iter = arr.into_iter();
    while let (Some(member), Some(score)) = (iter.next(), iter.next()) {
        let m = decode_value(member);
        let s = parse_score(score)?;
        pairs.push((m, s));
    }
    Ok(RedisValue::ZSet(pairs))
}

fn parse_score(v: RV) -> Result<f64> {
    match v {
        RV::Double(f) => Ok(f),
        RV::Int(i) => Ok(i as f64),
        RV::SimpleString(s) => s
            .parse::<f64>()
            .map_err(|e| DomainError::QueryFailed(format!("score 解析失败：{e}"))),
        RV::BulkString(bytes) => std::str::from_utf8(&bytes)
            .map_err(|e| DomainError::QueryFailed(format!("score 字节非 utf-8：{e}")))?
            .parse::<f64>()
            .map_err(|e| DomainError::QueryFailed(format!("score 解析失败：{e}"))),
        // BigNumber 携带 BigInt（精度超 i64），通过其 Display 转字符串再解析
        // 注意：极大数会损失精度，但 ZSCORE 不会返回此类值，仅是兜底
        RV::BigNumber(big) => big
            .to_string()
            .parse::<f64>()
            .map_err(|e| DomainError::QueryFailed(format!("score 解析失败：{e}"))),
        other => Err(DomainError::QueryFailed(format!(
            "score 应为数字，实得：{other:?}"
        ))),
    }
}

/// XRANGE / XREVRANGE 应答 → RedisValue::Stream
///
/// 应答结构：`Array([Array([id_bulk, Array([f, v, f, v, ...])]), ...])`
pub fn decode_stream_entries(v: RV) -> Result<RedisValue> {
    let entries = match v {
        RV::Array(a) => a,
        RV::Nil => return Ok(RedisValue::Nil),
        other => {
            return Err(DomainError::QueryFailed(format!(
                "XRANGE 应答非数组：{other:?}"
            )));
        }
    };

    let mut out: Vec<StreamEntry> = Vec::with_capacity(entries.len());
    for entry in entries {
        out.push(decode_stream_entry(entry)?);
    }
    Ok(RedisValue::Stream(out))
}

fn decode_stream_entry(v: RV) -> Result<StreamEntry> {
    let mut parts = match v {
        RV::Array(a) => a,
        other => {
            return Err(DomainError::QueryFailed(format!(
                "Stream entry 非数组：{other:?}"
            )));
        }
    };
    if parts.len() != 2 {
        return Err(DomainError::QueryFailed(format!(
            "Stream entry 期望 2 元素，实得 {}",
            parts.len()
        )));
    }
    let fields_raw = parts.remove(1);
    let id_raw = parts.remove(0);

    let id = match decode_value(id_raw) {
        RedisValue::Text(s) => s,
        other => other.display_preview(64),
    };

    let fields = decode_stream_fields(fields_raw)?;
    Ok(StreamEntry { id, fields })
}

fn decode_stream_fields(v: RV) -> Result<Vec<(String, String)>> {
    let arr = match v {
        RV::Array(a) => a,
        other => {
            return Err(DomainError::QueryFailed(format!(
                "Stream fields 非数组：{other:?}"
            )));
        }
    };
    if arr.len() % 2 != 0 {
        return Err(DomainError::QueryFailed(format!(
            "Stream fields 长度非偶数：{}",
            arr.len()
        )));
    }
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(arr.len() / 2);
    let mut iter = arr.into_iter();
    while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
        let key = stringify(decode_value(k));
        let val = stringify(decode_value(v));
        pairs.push((key, val));
    }
    Ok(pairs)
}

fn stringify(v: RedisValue) -> String {
    match v {
        RedisValue::Text(s) => s,
        RedisValue::Bytes(b) => String::from_utf8_lossy(&b).into_owned(),
        RedisValue::Int(i) => i.to_string(),
        RedisValue::Float(f) => f.to_string(),
        other => other.display_preview(128),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_int_and_nil() {
        assert!(matches!(decode_value(RV::Nil), RedisValue::Nil));
        assert!(matches!(decode_value(RV::Int(42)), RedisValue::Int(42)));
    }

    #[test]
    fn decode_utf8_bulk() {
        let v = decode_value(RV::BulkString(b"hello".to_vec()));
        assert!(matches!(v, RedisValue::Text(s) if s == "hello"));
    }

    #[test]
    fn decode_non_utf8_bulk_fallback_to_bytes() {
        let v = decode_value(RV::BulkString(vec![0xff, 0xfe, 0x00]));
        assert!(matches!(v, RedisValue::Bytes(b) if b == vec![0xff, 0xfe, 0x00]));
    }

    #[test]
    fn decode_hash_pairs_works() {
        let arr = RV::Array(vec![
            RV::BulkString(b"a".to_vec()),
            RV::BulkString(b"1".to_vec()),
            RV::BulkString(b"b".to_vec()),
            RV::BulkString(b"2".to_vec()),
        ]);
        let r = decode_hash_pairs(arr).unwrap();
        match r {
            RedisValue::Hash(pairs) => {
                assert_eq!(pairs.len(), 2);
                assert_eq!(pairs[0].0, "a");
                assert_eq!(pairs[1].0, "b");
            }
            other => panic!("期望 Hash，实得 {other:?}"),
        }
    }

    #[test]
    fn decode_zset_with_scores_works() {
        // 用非 π/e 近似的常量避开 clippy::approx_constant
        let alice_score: f64 = 1.5;
        let bob_score: f64 = 4.25;
        let arr = RV::Array(vec![
            RV::BulkString(b"alice".to_vec()),
            RV::BulkString(b"1.5".to_vec()),
            RV::BulkString(b"bob".to_vec()),
            RV::Double(bob_score),
        ]);
        let r = decode_zset_with_scores(arr).unwrap();
        match r {
            RedisValue::ZSet(pairs) => {
                assert_eq!(pairs.len(), 2);
                assert!((pairs[0].1 - alice_score).abs() < 1e-9);
                assert!((pairs[1].1 - bob_score).abs() < 1e-9);
            }
            other => panic!("期望 ZSet，实得 {other:?}"),
        }
    }

    #[test]
    fn decode_stream_entries_works() {
        let entry = RV::Array(vec![
            RV::BulkString(b"1234567890-0".to_vec()),
            RV::Array(vec![
                RV::BulkString(b"field1".to_vec()),
                RV::BulkString(b"value1".to_vec()),
            ]),
        ]);
        let stream = RV::Array(vec![entry]);
        let r = decode_stream_entries(stream).unwrap();
        match r {
            RedisValue::Stream(entries) => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].id, "1234567890-0");
                assert_eq!(entries[0].fields.len(), 1);
                assert_eq!(entries[0].fields[0], ("field1".into(), "value1".into()));
            }
            other => panic!("期望 Stream，实得 {other:?}"),
        }
    }

    #[test]
    fn decode_hash_odd_length_errors() {
        let arr = RV::Array(vec![
            RV::BulkString(b"a".to_vec()),
            RV::BulkString(b"1".to_vec()),
            RV::BulkString(b"b".to_vec()),
        ]);
        assert!(decode_hash_pairs(arr).is_err());
    }
}
