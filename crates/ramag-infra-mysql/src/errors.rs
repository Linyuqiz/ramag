//! sqlx::Error → DomainError 映射
//!
//! 把底层 sqlx 错误转换成 Domain 层统一的 DomainError，并尽可能识别
//! MySQL 特定错误码，给用户更友好的提示。
//!
//! 参考：<https://dev.mysql.com/doc/mysql-errors/8.0/en/server-error-reference.html>

use ramag_domain::error::DomainError;

/// 把 sqlx::Error 转成 DomainError
pub fn map_sqlx_error(err: sqlx::Error) -> DomainError {
    match &err {
        // === 网络/连接类 ===
        sqlx::Error::PoolTimedOut => {
            DomainError::ConnectionFailed("连接池等待超时（数据库可能繁忙或不可达）".into())
        }
        sqlx::Error::PoolClosed => DomainError::ConnectionFailed("连接池已关闭".into()),
        sqlx::Error::Io(io_err) => DomainError::ConnectionFailed(format!("网络/IO 错误：{io_err}")),
        sqlx::Error::Tls(tls_err) => DomainError::ConnectionFailed(format!("TLS 错误：{tls_err}")),

        // === MySQL 数据库错误（含错误码识别）===
        sqlx::Error::Database(db_err) => {
            let code = db_err.code().map(|c| c.to_string()).unwrap_or_default();
            let raw_msg = db_err.message().to_string();
            let friendly = mysql_error_friendly(&code, &raw_msg);

            match code.as_str() {
                "1045" => DomainError::ConnectionFailed(friendly), // Access denied
                "1049" => DomainError::ConnectionFailed(friendly), // Unknown database
                "2003" => DomainError::ConnectionFailed(friendly), // Can't connect
                "2005" => DomainError::ConnectionFailed(friendly), // Unknown host
                _ => DomainError::QueryFailed(friendly),
            }
        }

        // === 查询/解析类 ===
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

        // === 协议/配置类 ===
        sqlx::Error::Protocol(msg) => DomainError::ConnectionFailed(format!("协议错误：{msg}")),
        sqlx::Error::Configuration(e) => DomainError::InvalidConfig(format!("配置错误：{e}")),

        // === 兜底 ===
        _ => DomainError::Other(format!("sqlx 错误：{err}")),
    }
}

/// 把 MySQL 错误码 + 原始消息转成中文友好提示
///
/// 只覆盖 L0/L1 级别的常见错误，其他保留原始消息
fn mysql_error_friendly(code: &str, raw: &str) -> String {
    match code {
        "1045" => format!("用户名或密码错误（{raw}）"),
        "1049" => format!("数据库不存在（{raw}）"),
        "1054" => format!("字段不存在（{raw}）"),
        "1062" => format!("唯一键冲突（{raw}）"),
        "1064" => format!("SQL 语法错误（{raw}）"),
        "1067" => format!("默认值无效（{raw}）"),
        "1075" => format!("AUTO_INCREMENT 列定义无效（{raw}）"),
        "1142" => format!("权限不足，无法执行该操作（{raw}）"),
        "1146" => format!("表不存在（{raw}）"),
        "1205" => format!("锁等待超时（事务可能死锁，建议重试）：{raw}"),
        "1213" => format!("死锁（事务被强制回滚，请重试）：{raw}"),
        "1216" => format!("外键约束失败（{raw}）"),
        "1217" => format!("外键约束阻止删除（{raw}）"),
        "1264" => format!("字段值越界（{raw}）"),
        "1265" => format!("字段被截断（数据超出列长度）：{raw}"),
        "1267" => format!("字符集冲突（{raw}）"),
        "1366" => format!("数据格式不匹配（类型/编码错误）：{raw}"),
        "1406" => format!("字段值过长，超出列定义长度（{raw}）"),
        "1452" => format!("外键引用的记录不存在（{raw}）"),
        "1690" => format!("数值类型越界（{raw}）"),
        "1927" => format!("查询被取消（KILL QUERY）：{raw}"),
        "2003" => format!("无法连接到 MySQL 服务器（检查 host/port/防火墙）：{raw}"),
        "2005" => format!("无法解析主机名：{raw}"),
        "2006" => format!("MySQL 连接已断开（gone away）：{raw}"),
        "2013" => format!("查询期间连接断开：{raw}"),
        _ => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_known_codes() {
        assert!(mysql_error_friendly("1045", "Access denied").contains("用户名或密码"));
        assert!(mysql_error_friendly("1146", "Table 'x' doesn't exist").contains("表不存在"));
        assert!(mysql_error_friendly("1062", "Duplicate entry").contains("唯一键冲突"));
    }

    #[test]
    fn friendly_unknown_code_returns_raw() {
        let raw = "some unknown error";
        assert_eq!(mysql_error_friendly("9999", raw), raw);
    }
}
