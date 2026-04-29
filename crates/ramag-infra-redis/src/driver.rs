//! RedisDriver：实现 KvDriver trait
//!
//! 所有 trait 方法的实现模式：
//! 1. clone 配置 + 拿 PoolCache 句柄
//! 2. `run_in_tokio` 派发到 tokio runtime（GPUI smol 上下文也能 await）
//! 3. 在 tokio 内：取连接管理器 → 发命令 → 解码应答 → 映射错误
//!
//! 内部辅助函数都接收 `&mut ConnectionManager`（克隆出独立句柄后传入），
//! 不污染缓存。

use async_trait::async_trait;
use ramag_domain::entities::{ConnectionConfig, KeyMeta, RedisType, RedisValue, ScanResult};
use ramag_domain::error::{DomainError, Result};
use ramag_domain::traits::KvDriver;
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, Cmd, Value as RV};
use tracing::debug;

use crate::errors::map_redis_error;
use crate::pool::PoolCache;
use crate::runtime::run_in_tokio;
use crate::value::{
    decode_hash_pairs, decode_stream_entries, decode_value, decode_zset_with_scores,
};

/// Redis 驱动
pub struct RedisDriver {
    pools: PoolCache,
}

impl RedisDriver {
    pub fn new() -> Self {
        Self {
            pools: PoolCache::new(),
        }
    }

    /// 配置变更后调用，强制下次重建连接池（含该连接的所有 db 缓存）
    pub fn evict_pool(&self, id: &ramag_domain::entities::ConnectionId) {
        self.pools.evict_all_dbs(id);
    }
}

impl Default for RedisDriver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl KvDriver for RedisDriver {
    fn name(&self) -> &'static str {
        "redis"
    }

    async fn test_connection(&self, config: &ConnectionConfig) -> Result<()> {
        let config = config.clone();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let mut mgr = pools.get_or_create(&config, 0).await?;
            ping(&mut mgr).await
        })
        .await
    }

    async fn server_version(&self, config: &ConnectionConfig) -> Result<String> {
        let config = config.clone();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let mut mgr = pools.get_or_create(&config, 0).await?;
            let info = run_info(&mut mgr, &["server"]).await?;
            Ok(parse_redis_version(&info))
        })
        .await
    }

    async fn db_size(&self, config: &ConnectionConfig, db: u8) -> Result<u64> {
        let config = config.clone();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let mut mgr = pools.get_or_create(&config, db).await?;
            let n: u64 = redis::cmd("DBSIZE")
                .query_async(&mut mgr)
                .await
                .map_err(map_redis_error)?;
            Ok(n)
        })
        .await
    }

    async fn scan(
        &self,
        config: &ConnectionConfig,
        db: u8,
        cursor: u64,
        match_pattern: Option<&str>,
        type_filter: Option<RedisType>,
        count: u32,
    ) -> Result<ScanResult> {
        let config = config.clone();
        let pools = self.pools.clone_handle();
        let pattern = match_pattern.map(str::to_owned);
        run_in_tokio(async move {
            let mut mgr = pools.get_or_create(&config, db).await?;
            let mut cmd = redis::cmd("SCAN");
            cmd.arg(cursor);
            if let Some(p) = pattern.as_ref() {
                cmd.arg("MATCH").arg(p);
            }
            // SCAN COUNT 是 hint：传 1 也合法，redis 自行决定本批返回多少
            cmd.arg("COUNT").arg(count.max(1));
            if let Some(t) = type_filter {
                cmd.arg("TYPE").arg(t.as_scan_arg());
            }
            let v: RV = cmd.query_async(&mut mgr).await.map_err(map_redis_error)?;
            parse_scan_response(v)
        })
        .await
    }

    async fn key_type(&self, config: &ConnectionConfig, db: u8, key: &str) -> Result<RedisType> {
        let config = config.clone();
        let pools = self.pools.clone_handle();
        let key = key.to_owned();
        run_in_tokio(async move {
            let mut mgr = pools.get_or_create(&config, db).await?;
            let s: String = redis::cmd("TYPE")
                .arg(&key)
                .query_async(&mut mgr)
                .await
                .map_err(map_redis_error)?;
            Ok(RedisType::parse(&s))
        })
        .await
    }

    async fn key_ttl(&self, config: &ConnectionConfig, db: u8, key: &str) -> Result<i64> {
        let config = config.clone();
        let pools = self.pools.clone_handle();
        let key = key.to_owned();
        run_in_tokio(async move {
            let mut mgr = pools.get_or_create(&config, db).await?;
            let ms: i64 = redis::cmd("PTTL")
                .arg(&key)
                .query_async(&mut mgr)
                .await
                .map_err(map_redis_error)?;
            Ok(ms)
        })
        .await
    }

    async fn get_value(&self, config: &ConnectionConfig, db: u8, key: &str) -> Result<RedisValue> {
        let config = config.clone();
        let pools = self.pools.clone_handle();
        let key = key.to_owned();
        run_in_tokio(async move {
            let mut mgr = pools.get_or_create(&config, db).await?;
            // 先 TYPE，再按类型 dispatch
            let t: String = redis::cmd("TYPE")
                .arg(&key)
                .query_async(&mut mgr)
                .await
                .map_err(map_redis_error)?;
            let kind = RedisType::parse(&t);
            debug!(?key, ?kind, "get_value dispatch");
            match kind {
                RedisType::None => Ok(RedisValue::Nil),
                RedisType::String => fetch_string(&mut mgr, &key).await,
                RedisType::List => fetch_list(&mut mgr, &key).await,
                RedisType::Hash => fetch_hash(&mut mgr, &key).await,
                RedisType::Set => fetch_set(&mut mgr, &key).await,
                RedisType::ZSet => fetch_zset(&mut mgr, &key).await,
                RedisType::Stream => fetch_stream(&mut mgr, &key).await,
            }
        })
        .await
    }

    async fn delete_key(&self, config: &ConnectionConfig, db: u8, key: &str) -> Result<bool> {
        let config = config.clone();
        let pools = self.pools.clone_handle();
        let key = key.to_owned();
        run_in_tokio(async move {
            let mut mgr = pools.get_or_create(&config, db).await?;
            let removed: u32 = mgr.del(&key).await.map_err(map_redis_error)?;
            Ok(removed > 0)
        })
        .await
    }

    async fn set_ttl(
        &self,
        config: &ConnectionConfig,
        db: u8,
        key: &str,
        ttl_secs: Option<i64>,
    ) -> Result<bool> {
        let config = config.clone();
        let pools = self.pools.clone_handle();
        let key = key.to_owned();
        run_in_tokio(async move {
            let mut mgr = pools.get_or_create(&config, db).await?;
            let ok: i64 = match ttl_secs {
                Some(secs) => redis::cmd("EXPIRE")
                    .arg(&key)
                    .arg(secs)
                    .query_async(&mut mgr)
                    .await
                    .map_err(map_redis_error)?,
                None => redis::cmd("PERSIST")
                    .arg(&key)
                    .query_async(&mut mgr)
                    .await
                    .map_err(map_redis_error)?,
            };
            Ok(ok == 1)
        })
        .await
    }

    async fn execute_command(
        &self,
        config: &ConnectionConfig,
        db: u8,
        argv: Vec<String>,
    ) -> Result<RedisValue> {
        if argv.is_empty() {
            return Err(DomainError::InvalidConfig(
                "命令为空，至少需要命令名".into(),
            ));
        }
        let config = config.clone();
        let pools = self.pools.clone_handle();
        run_in_tokio(async move {
            let mut mgr = pools.get_or_create(&config, db).await?;
            let mut cmd = Cmd::new();
            for a in argv {
                cmd.arg(a);
            }
            let v: RV = cmd.query_async(&mut mgr).await.map_err(map_redis_error)?;
            Ok(decode_value(v))
        })
        .await
    }

    async fn info(&self, config: &ConnectionConfig, sections: &[&str]) -> Result<String> {
        let config = config.clone();
        let pools = self.pools.clone_handle();
        let sections: Vec<String> = sections.iter().map(|s| s.to_string()).collect();
        run_in_tokio(async move {
            let mut mgr = pools.get_or_create(&config, 0).await?;
            let refs: Vec<&str> = sections.iter().map(String::as_str).collect();
            run_info(&mut mgr, &refs).await
        })
        .await
    }

}

// === 内部命令封装 ===

async fn ping(mgr: &mut ConnectionManager) -> Result<()> {
    let pong: String = redis::cmd("PING")
        .query_async(mgr)
        .await
        .map_err(map_redis_error)?;
    if pong.eq_ignore_ascii_case("PONG") {
        Ok(())
    } else {
        Err(DomainError::ConnectionFailed(format!(
            "PING 应答异常：{pong}"
        )))
    }
}

async fn run_info(mgr: &mut ConnectionManager, sections: &[&str]) -> Result<String> {
    let mut cmd = redis::cmd("INFO");
    for s in sections {
        cmd.arg(*s);
    }
    let s: String = cmd.query_async(mgr).await.map_err(map_redis_error)?;
    Ok(s)
}

/// 从 INFO server 文本里提取 redis_version 字段
fn parse_redis_version(info: &str) -> String {
    for line in info.lines() {
        if let Some(rest) = line.strip_prefix("redis_version:") {
            return rest.trim().to_string();
        }
    }
    "unknown".into()
}

async fn fetch_string(mgr: &mut ConnectionManager, key: &str) -> Result<RedisValue> {
    let v: RV = redis::cmd("GET")
        .arg(key)
        .query_async(mgr)
        .await
        .map_err(map_redis_error)?;
    Ok(decode_value(v))
}

async fn fetch_list(mgr: &mut ConnectionManager, key: &str) -> Result<RedisValue> {
    let v: RV = redis::cmd("LRANGE")
        .arg(key)
        .arg(0)
        .arg(-1)
        .query_async(mgr)
        .await
        .map_err(map_redis_error)?;
    let elems = match v {
        RV::Array(a) => a.into_iter().map(decode_value).collect(),
        RV::Nil => return Ok(RedisValue::Nil),
        other => {
            return Err(DomainError::QueryFailed(format!(
                "LRANGE 应答非数组：{other:?}"
            )));
        }
    };
    Ok(RedisValue::List(elems))
}

async fn fetch_hash(mgr: &mut ConnectionManager, key: &str) -> Result<RedisValue> {
    let v: RV = redis::cmd("HGETALL")
        .arg(key)
        .query_async(mgr)
        .await
        .map_err(map_redis_error)?;
    decode_hash_pairs(v)
}

async fn fetch_set(mgr: &mut ConnectionManager, key: &str) -> Result<RedisValue> {
    let v: RV = redis::cmd("SMEMBERS")
        .arg(key)
        .query_async(mgr)
        .await
        .map_err(map_redis_error)?;
    let elems = match v {
        RV::Array(a) => a.into_iter().map(decode_value).collect(),
        RV::Set(a) => a.into_iter().map(decode_value).collect(),
        RV::Nil => return Ok(RedisValue::Nil),
        other => {
            return Err(DomainError::QueryFailed(format!(
                "SMEMBERS 应答非数组：{other:?}"
            )));
        }
    };
    Ok(RedisValue::Set(elems))
}

async fn fetch_zset(mgr: &mut ConnectionManager, key: &str) -> Result<RedisValue> {
    let v: RV = redis::cmd("ZRANGE")
        .arg(key)
        .arg(0)
        .arg(-1)
        .arg("WITHSCORES")
        .query_async(mgr)
        .await
        .map_err(map_redis_error)?;
    decode_zset_with_scores(v)
}

async fn fetch_stream(mgr: &mut ConnectionManager, key: &str) -> Result<RedisValue> {
    let v: RV = redis::cmd("XRANGE")
        .arg(key)
        .arg("-")
        .arg("+")
        .query_async(mgr)
        .await
        .map_err(map_redis_error)?;
    decode_stream_entries(v)
}

/// SCAN 应答：`Array([cursor_str, Array([key1, key2, ...])])`
fn parse_scan_response(v: RV) -> Result<ScanResult> {
    let mut top = match v {
        RV::Array(a) => a,
        other => {
            return Err(DomainError::QueryFailed(format!(
                "SCAN 应答非数组：{other:?}"
            )));
        }
    };
    if top.len() != 2 {
        return Err(DomainError::QueryFailed(format!(
            "SCAN 应答应有 2 元素，实得 {}",
            top.len()
        )));
    }
    let keys_raw = top.remove(1);
    let cursor_raw = top.remove(0);

    let cursor = match cursor_raw {
        RV::BulkString(bytes) => std::str::from_utf8(&bytes)
            .map_err(|e| DomainError::QueryFailed(format!("SCAN cursor 非 utf-8：{e}")))?
            .parse::<u64>()
            .map_err(|e| DomainError::QueryFailed(format!("SCAN cursor 非数字：{e}")))?,
        RV::SimpleString(s) => s
            .parse::<u64>()
            .map_err(|e| DomainError::QueryFailed(format!("SCAN cursor 非数字：{e}")))?,
        RV::Int(i) => i as u64,
        other => {
            return Err(DomainError::QueryFailed(format!(
                "SCAN cursor 类型异常：{other:?}"
            )));
        }
    };

    let key_arr = match keys_raw {
        RV::Array(a) => a,
        other => {
            return Err(DomainError::QueryFailed(format!(
                "SCAN keys 非数组：{other:?}"
            )));
        }
    };

    let keys: Vec<KeyMeta> = key_arr
        .into_iter()
        .filter_map(|v| match decode_value(v) {
            RedisValue::Text(s) => Some(KeyMeta::bare(s)),
            RedisValue::Bytes(b) => Some(KeyMeta::bare(String::from_utf8_lossy(&b).into_owned())),
            _ => None,
        })
        .collect();

    Ok(ScanResult { cursor, keys })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_finds_field() {
        let info = "# Server\r\nredis_version:7.2.4\r\nredis_mode:standalone\r\n";
        assert_eq!(parse_redis_version(info), "7.2.4");
    }

    #[test]
    fn parse_version_missing_returns_unknown() {
        assert_eq!(parse_redis_version("# Server\r\nfoo:bar\r\n"), "unknown");
    }

    #[test]
    fn parse_scan_basic() {
        let v = RV::Array(vec![
            RV::BulkString(b"123".to_vec()),
            RV::Array(vec![
                RV::BulkString(b"key1".to_vec()),
                RV::BulkString(b"key2".to_vec()),
            ]),
        ]);
        let r = parse_scan_response(v).unwrap();
        assert_eq!(r.cursor, 123);
        assert_eq!(r.keys.len(), 2);
        assert_eq!(r.keys[0].key, "key1");
        assert_eq!(r.keys[1].key, "key2");
    }

    #[test]
    fn parse_scan_end_cursor_zero() {
        let v = RV::Array(vec![RV::BulkString(b"0".to_vec()), RV::Array(vec![])]);
        let r = parse_scan_response(v).unwrap();
        assert_eq!(r.cursor, 0);
        assert!(r.keys.is_empty());
    }
}
