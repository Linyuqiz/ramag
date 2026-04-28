//! redis::RedisError → DomainError 映射
//!
//! 把底层 redis-rs 错误转换成 Domain 层统一的 DomainError，并尽可能识别
//! Redis 特定错误前缀（NOAUTH / WRONGTYPE / OOM / READONLY 等），
//! 给用户更友好的中文提示。
//!
//! 参考：<https://redis.io/docs/reference/error-handling/>

use ramag_domain::error::DomainError;
use redis::{ErrorKind, RedisError};

/// 把 redis::RedisError 转成 DomainError
pub fn map_redis_error(err: RedisError) -> DomainError {
    let kind = err.kind();
    let code = err.code().unwrap_or("");
    let detail = err.detail().unwrap_or("");
    let raw = err.to_string();

    match kind {
        // 认证类
        ErrorKind::AuthenticationFailed => {
            DomainError::ConnectionFailed(format!("认证失败（密码或 ACL 凭证错误）：{raw}"))
        }
        ErrorKind::IoError => DomainError::ConnectionFailed(format!("网络/IO 错误：{raw}")),
        ErrorKind::ClientError => DomainError::ConnectionFailed(format!("客户端错误：{raw}")),

        // 类型 / 应答类
        ErrorKind::TypeError => {
            DomainError::QueryFailed(format!("类型错误（应答与期望不匹配）：{raw}"))
        }
        ErrorKind::ExecAbortError => {
            DomainError::QueryFailed(format!("MULTI/EXEC 事务被中止：{raw}"))
        }
        ErrorKind::ResponseError | ErrorKind::ExtensionError => {
            DomainError::QueryFailed(redis_response_friendly(code, detail, &raw))
        }

        // 集群 / 脚本 / 解析
        ErrorKind::Moved | ErrorKind::Ask => {
            DomainError::QueryFailed(format!("Cluster 重定向（{code}），客户端未透明跟随：{raw}"))
        }
        ErrorKind::TryAgain => {
            DomainError::QueryFailed(format!("Cluster 槽位迁移中，请稍后重试：{raw}"))
        }
        ErrorKind::ClusterDown => {
            DomainError::ConnectionFailed(format!("Cluster 不可用（部分主节点 down）：{raw}"))
        }
        ErrorKind::CrossSlot => {
            DomainError::QueryFailed(format!("命令跨槽位，需在同一 hash slot 内：{raw}"))
        }
        ErrorKind::MasterDown => DomainError::ConnectionFailed(format!("主节点不可用：{raw}")),
        ErrorKind::ReadOnly => DomainError::QueryFailed(format!(
            "只读副本拒绝写入（READONLY），请连接到主节点：{raw}"
        )),
        ErrorKind::NoScriptError => DomainError::QueryFailed(format!(
            "脚本未缓存（NOSCRIPT），请改用 EVAL 重新加载：{raw}"
        )),
        ErrorKind::BusyLoadingError => DomainError::ConnectionFailed(format!(
            "服务端正在加载 RDB（LOADING），请稍后再试：{raw}"
        )),
        ErrorKind::InvalidClientConfig => {
            DomainError::InvalidConfig(format!("客户端配置无效：{raw}"))
        }

        // 兜底
        _ => DomainError::Other(format!("redis 错误：{raw}")),
    }
}

/// Redis 应答错误 code（如 NOAUTH / WRONGTYPE / OOM）→ 中文友好提示
fn redis_response_friendly(code: &str, detail: &str, raw: &str) -> String {
    let body = if detail.is_empty() { raw } else { detail };
    match code {
        "NOAUTH" => format!("未认证，请先 AUTH 或在连接配置中填密码（{body}）"),
        "WRONGPASS" => format!("用户名或密码错误（{body}）"),
        "WRONGTYPE" => format!("Key 类型与命令不匹配（{body}）"),
        "OOM" => format!("Redis 已达 maxmemory 上限，写入被拒绝（{body}）"),
        "BUSY" => format!("有 Lua 脚本/复制操作在执行，必要时 SCRIPT KILL（{body}）"),
        "LOADING" => format!("Redis 正在加载持久化数据，请稍后（{body}）"),
        "READONLY" => format!("只读副本拒绝写入，请连接主节点（{body}）"),
        "NOSCRIPT" => format!("脚本未缓存，请改用 EVAL 全文加载（{body}）"),
        "MOVED" | "ASK" => format!("Cluster 重定向（{code}），客户端应自动跟随（{body}）"),
        "MASTERDOWN" => format!("主节点不可用（{body}）"),
        "CLUSTERDOWN" => format!("Cluster 不可用（{body}）"),
        "TRYAGAIN" => format!("槽位迁移中，请稍后重试（{body}）"),
        "" => body.to_string(),
        _ => format!("{code}：{body}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_known_codes() {
        assert!(redis_response_friendly("NOAUTH", "auth required", "").contains("未认证"));
        assert!(
            redis_response_friendly("WRONGTYPE", "wrong kind", "").contains("类型与命令不匹配")
        );
        assert!(redis_response_friendly("OOM", "memory full", "").contains("maxmemory"));
    }

    #[test]
    fn friendly_unknown_returns_code_with_detail() {
        assert!(redis_response_friendly("FOO", "bar", "").starts_with("FOO："));
    }

    #[test]
    fn friendly_empty_code_returns_detail() {
        assert_eq!(redis_response_friendly("", "abc", "raw"), "abc");
    }
}
