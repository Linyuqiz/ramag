//! Redis 写命令识别：生产模式只读保护用。
//! 黑名单覆盖全部主流写命令；带可选写参数的命令（SORT / EVAL / GEORADIUS / BITFIELD / GETEX）
//! 保守归写，其 `_RO` 只读变体不在名单内，自动放行。
//! 管理类命令（CONFIG / CLUSTER / ACL / SCRIPT 等）整体归写——子命令不细分，
//! 少量只读子命令被一并拦截，但生产只读场景本就不应使用，符合保守封死原则

/// 命令名（大小写不敏感）是否为写命令
pub fn is_write_command(cmd: &str) -> bool {
    let upper = cmd.to_ascii_uppercase();
    WRITE_COMMANDS.contains(&upper.as_str())
}

const WRITE_COMMANDS: &[&str] = &[
    // String / 通用
    "SET",
    "SETNX",
    "SETEX",
    "PSETEX",
    "SETRANGE",
    "APPEND",
    "GETSET",
    "GETDEL",
    "GETEX",
    "MSET",
    "MSETNX",
    "INCR",
    "DECR",
    "INCRBY",
    "DECRBY",
    "INCRBYFLOAT",
    "DEL",
    "UNLINK",
    "EXPIRE",
    "PEXPIRE",
    "EXPIREAT",
    "PEXPIREAT",
    "PERSIST",
    "RENAME",
    "RENAMENX",
    "MOVE",
    "COPY",
    "RESTORE",
    "MIGRATE",
    // List
    "LPUSH",
    "RPUSH",
    "LPUSHX",
    "RPUSHX",
    "LPOP",
    "RPOP",
    "LSET",
    "LINSERT",
    "LREM",
    "LTRIM",
    "RPOPLPUSH",
    "LMOVE",
    "BLPOP",
    "BRPOP",
    "BLMOVE",
    "BRPOPLPUSH",
    "LMPOP",
    "BLMPOP",
    // Set
    "SADD",
    "SREM",
    "SPOP",
    "SMOVE",
    "SINTERSTORE",
    "SUNIONSTORE",
    "SDIFFSTORE",
    // Hash
    "HSET",
    "HSETNX",
    "HMSET",
    "HDEL",
    "HINCRBY",
    "HINCRBYFLOAT",
    "HEXPIRE",
    "HPEXPIRE",
    "HEXPIREAT",
    "HPEXPIREAT",
    "HPERSIST",
    // ZSet
    "ZADD",
    "ZREM",
    "ZINCRBY",
    "ZPOPMIN",
    "ZPOPMAX",
    "BZPOPMIN",
    "BZPOPMAX",
    "ZREMRANGEBYRANK",
    "ZREMRANGEBYSCORE",
    "ZREMRANGEBYLEX",
    "ZRANGESTORE",
    "ZDIFFSTORE",
    "ZINTERSTORE",
    "ZUNIONSTORE",
    "ZMPOP",
    "BZMPOP",
    // Stream
    "XADD",
    "XDEL",
    "XTRIM",
    "XSETID",
    "XGROUP",
    "XCLAIM",
    "XAUTOCLAIM",
    "XACK",
    "XREADGROUP",
    // HyperLogLog
    "PFADD",
    "PFMERGE",
    // Geo（带 STORE 写，保守归写）
    "GEOADD",
    "GEOSEARCHSTORE",
    "GEORADIUS",
    "GEORADIUSBYMEMBER",
    // Bitmap
    "SETBIT",
    "BITOP",
    "BITFIELD",
    // Scripting（可能写，保守归写；_RO 变体放行）
    "EVAL",
    "EVALSHA",
    "FCALL",
    // 排序（带 STORE 写；SORT_RO 放行）
    "SORT",
    // Server / 管理（改状态 / 危险）
    "FLUSHDB",
    "FLUSHALL",
    "SWAPDB",
    "CONFIG",
    "FUNCTION",
    "SCRIPT",
    "DEBUG",
    "SHUTDOWN",
    "SAVE",
    "BGSAVE",
    "BGREWRITEAOF",
    "SLAVEOF",
    "REPLICAOF",
    "FAILOVER",
    "RESET",
    "ACL",
    "CLUSTER",
    "LATENCY",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_detected() {
        for c in [
            "SET", "del", "HSET", "ZADD", "XADD", "FLUSHDB", "flushall", "expire", "rename",
            "lpush", "getex", "sort", "eval", "config",
        ] {
            assert!(is_write_command(c), "{c} 应判为写命令");
        }
    }

    #[test]
    fn reads_allowed() {
        for c in [
            "GET",
            "mget",
            "HGETALL",
            "LRANGE",
            "SCAN",
            "TYPE",
            "TTL",
            "PTTL",
            "ZRANGE",
            "XRANGE",
            "INFO",
            "PING",
            "EXISTS",
            "SMEMBERS",
            "DBSIZE",
            "KEYS",
            "HSCAN",
            "PFCOUNT",
            // _RO 只读变体须放行
            "SORT_RO",
            "EVAL_RO",
            "EVALSHA_RO",
            "BITFIELD_RO",
            "GEORADIUS_RO",
        ] {
            assert!(!is_write_command(c), "{c} 应判为只读命令");
        }
    }
}
