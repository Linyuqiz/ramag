//! sqlx::Error 通用大类映射。driver 先识别 SQLSTATE/errno，再 fallback 到这里

use ramag_domain::error::DomainError;

pub fn map_sqlx_common(err: &sqlx::Error) -> DomainError {
    match err {
        sqlx::Error::PoolTimedOut => {
            DomainError::ConnectionFailed("连接池等待超时（数据库可能繁忙或不可达）".into())
        }
        sqlx::Error::PoolClosed => DomainError::ConnectionFailed("连接池已关闭".into()),
        sqlx::Error::Io(io) => DomainError::ConnectionFailed(format!("网络/IO 错误：{io}")),
        sqlx::Error::Tls(tls) => DomainError::ConnectionFailed(format!("TLS 错误：{tls}")),

        sqlx::Error::ColumnDecode { index, source } => {
            DomainError::QueryFailed(format!("列解码失败（第 {index} 列）：{source}"))
        }
        sqlx::Error::Decode(e) => DomainError::QueryFailed(format!("数据解码失败：{e}")),
        sqlx::Error::TypeNotFound { type_name } => {
            DomainError::QueryFailed(format!("类型未识别：{type_name}"))
        }
        sqlx::Error::ColumnNotFound(name) => DomainError::QueryFailed(format!("列不存在：{name}")),
        sqlx::Error::ColumnIndexOutOfBounds { index, len } => {
            DomainError::QueryFailed(format!("列索引越界：{index} ≥ {len}"))
        }
        sqlx::Error::RowNotFound => DomainError::NotFound("查询结果为空".into()),

        sqlx::Error::Protocol(msg) => DomainError::ConnectionFailed(format!("协议错误：{msg}")),
        sqlx::Error::Configuration(e) => DomainError::InvalidConfig(format!("配置错误：{e}")),

        _ => DomainError::Other(format!("sqlx 错误：{err}")),
    }
}
