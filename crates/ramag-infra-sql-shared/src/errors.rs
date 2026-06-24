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

/// Database 错误映射骨架：取 code/message → friendly → classify 成大类。
/// 非 Database 变体返回 None（调用方决定是否 fallback 到 [`map_sqlx_common`]）。
/// - `friendly`: (code, raw_msg) → 中文友好消息
/// - `classify`: (code, friendly_msg) → DomainError 大类（方言各自的码表）
pub fn map_database_error(
    err: &sqlx::Error,
    friendly: impl Fn(&str, &str) -> String,
    classify: impl Fn(&str, String) -> DomainError,
) -> Option<DomainError> {
    let db_err = err.as_database_error()?;
    let code = db_err.code().map(|c| c.to_string()).unwrap_or_default();
    let raw_msg = db_err.message().to_string();
    let msg = friendly(&code, &raw_msg);
    Some(classify(&code, msg))
}
