//! MySQL 错误码 → DomainError 映射
//!
//! 通用 sqlx 大类（Pool / Io / Tls / Decode 等）由 `sql-shared::errors::map_sqlx_common`
//! 兜底；本模块识别 MySQL 数据库错误码（sqlx::Error::Database），其它返回 None 让上层兜底
//!
//! 参考：<https://dev.mysql.com/doc/mysql-errors/8.0/en/server-error-reference.html>

use ramag_domain::error::DomainError;
use ramag_infra_sql_shared::errors::map_sqlx_common;

/// 入口：sqlx::Error → DomainError（先 mysql 错误码、再 shared 通用）
pub fn map_mysql_error(err: &sqlx::Error) -> DomainError {
    map_mysql_database_error(err).unwrap_or_else(|| map_sqlx_common(err))
}

/// 仅识别 mysql 数据库错误码；非 Database 变体返回 None 让上层走通用兜底
pub fn map_mysql_database_error(err: &sqlx::Error) -> Option<DomainError> {
    let db_err = err.as_database_error()?;
    let code = db_err.code().map(|c| c.to_string()).unwrap_or_default();
    let raw_msg = db_err.message().to_string();
    let friendly = mysql_error_friendly(&code, &raw_msg);

    Some(match code.as_str() {
        // 网络/认证类归到 ConnectionFailed
        "1045" | "1049" | "2003" | "2005" => DomainError::ConnectionFailed(friendly),
        _ => DomainError::QueryFailed(friendly),
    })
}

/// MySQL 错误码 + 原始消息 → 中文友好提示
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
        assert_eq!(mysql_error_friendly("9999", "raw msg"), "raw msg");
    }
}
