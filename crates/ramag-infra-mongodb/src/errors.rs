//! mongodb::error::Error → DomainError。按 ErrorKind 大类映射，识别认证 / 网络 / 命令等典型场景

use mongodb::error::{Error as MongoError, ErrorKind};
use ramag_domain::error::DomainError;

pub fn map_mongo_error(err: MongoError) -> DomainError {
    let raw = err.to_string();

    // ErrorKind 是 non_exhaustive，必须用 `_` 兜底
    match err.kind.as_ref() {
        ErrorKind::Authentication { .. } => DomainError::ConnectionFailed(format!(
            "认证失败（用户名 / 密码 / authSource 错误）：{raw}"
        )),
        ErrorKind::Io(_) => DomainError::ConnectionFailed(format!("网络 / IO 错误：{raw}")),
        ErrorKind::ServerSelection { .. } => {
            DomainError::ConnectionFailed(format!("无法选择服务端（请检查 host/port/TLS）：{raw}"))
        }
        ErrorKind::ConnectionPoolCleared { .. } => {
            DomainError::ConnectionFailed(format!("连接池被清空：{raw}"))
        }
        ErrorKind::DnsResolve { .. } => {
            DomainError::ConnectionFailed(format!("DNS 解析失败：{raw}"))
        }
        ErrorKind::Command(cmd) => DomainError::QueryFailed(format!(
            "命令错误（code={}, name={}）：{raw}",
            cmd.code, cmd.code_name
        )),
        ErrorKind::Write(_) => DomainError::QueryFailed(format!("写入错误：{raw}")),
        ErrorKind::BulkWrite(_) => DomainError::QueryFailed(format!("批量写入错误：{raw}")),
        ErrorKind::InvalidArgument { .. } => DomainError::InvalidConfig(format!("参数错误：{raw}")),
        ErrorKind::InvalidResponse { .. } => {
            DomainError::QueryFailed(format!("服务端响应无效：{raw}"))
        }
        ErrorKind::BsonDeserialization(_) | ErrorKind::BsonSerialization(_) => {
            DomainError::QueryFailed(format!("BSON 序列化失败：{raw}"))
        }
        _ => DomainError::Other(format!("mongodb 错误：{raw}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 简单 smoke：随便造一个 Mongo error 看映射不 panic
    #[test]
    fn map_does_not_panic() {
        let err: MongoError = MongoError::custom("smoke");
        let mapped = map_mongo_error(err);
        let msg = format!("{mapped}");
        assert!(!msg.is_empty());
    }
}
