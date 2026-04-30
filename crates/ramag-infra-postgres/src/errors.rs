//! PostgreSQL SQLSTATE → DomainError 映射
//!
//! 通用 sqlx 大类（Pool / Io / Tls / Decode）由 `sql-shared::errors::map_sqlx_common`
//! 兜底；本模块识别 PG 数据库 SQLSTATE（5 字符代码），其他变体返回 None 让上层兜底
//!
//! SQLSTATE 类码（前 2 字符）含义：
//! - `08xxx`：连接异常
//! - `23xxx`：完整性约束违反（unique/foreign_key/check/not_null）
//! - `25xxx`：事务状态错误
//! - `28xxx`：认证错误（密码错 / 鉴权失败）
//! - `42xxx`：语法 / 权限 / 标识符
//! - `53xxx`：资源不足
//! - `57014`：query_canceled

use ramag_domain::error::DomainError;
use ramag_infra_sql_shared::errors::map_sqlx_common;

/// 入口：sqlx::Error → DomainError（先 PG SQLSTATE、再 shared 通用）
pub fn map_postgres_error(err: &sqlx::Error) -> DomainError {
    map_postgres_database_error(err).unwrap_or_else(|| map_sqlx_common(err))
}

/// 仅识别 PG SQLSTATE；非 Database 变体返回 None 让上层走通用兜底
pub fn map_postgres_database_error(err: &sqlx::Error) -> Option<DomainError> {
    let db_err = err.as_database_error()?;
    let code = db_err.code().map(|c| c.to_string()).unwrap_or_default();
    let raw_msg = db_err.message().to_string();
    let friendly = postgres_error_friendly(&code, &raw_msg);

    // 类码前 2 位决定 DomainError 大类
    let class = code.get(..2).unwrap_or("");
    Some(match class {
        // 08xxx 连接异常 / 28xxx 认证失败 → ConnectionFailed
        "08" | "28" => DomainError::ConnectionFailed(friendly),
        _ => DomainError::QueryFailed(friendly),
    })
}

/// SQLSTATE 5 字符代码 + 原始消息 → 中文友好提示
fn postgres_error_friendly(code: &str, raw: &str) -> String {
    match code {
        // 08xxx 连接异常
        "08000" => format!("连接异常（{raw}）"),
        "08003" => format!("连接不存在（{raw}）"),
        "08006" => format!("连接失败（{raw}）"),
        "08001" => format!("无法建立连接（检查 host/port/防火墙）：{raw}"),
        "08004" => format!("服务器拒绝连接（{raw}）"),

        // 23xxx 完整性约束
        "23502" => format!("非空约束违反（NOT NULL）：{raw}"),
        "23503" => format!("外键约束违反：{raw}"),
        "23505" => format!("唯一键冲突：{raw}"),
        "23514" => format!("CHECK 约束违反：{raw}"),
        "23P01" => format!("EXCLUSION 约束违反：{raw}"),

        // 25xxx 事务状态
        "25001" => format!("事务里只能跑一条语句（{raw}）"),
        "25P02" => format!("事务已 abort，请 ROLLBACK 后重试：{raw}"),
        "25006" => format!("只读事务中不允许写：{raw}"),

        // 28xxx 认证
        "28000" => format!("鉴权失败（{raw}）"),
        "28P01" => format!("用户名或密码错误：{raw}"),

        // 42xxx 语法 / 权限
        "42000" => format!("语法或权限错误（{raw}）"),
        "42501" => format!("权限不足：{raw}"),
        "42601" => format!("SQL 语法错误：{raw}"),
        "42703" => format!("字段不存在：{raw}"),
        "42883" => format!("函数不存在：{raw}"),
        "42P01" => format!("表/视图不存在：{raw}"),
        "42P02" => format!("参数不存在：{raw}"),
        "42P07" => format!("对象已存在：{raw}"),

        // 53xxx 资源
        "53100" => format!("磁盘满（{raw}）"),
        "53200" => format!("内存不足（{raw}）"),
        "53300" => format!("连接数已达上限（{raw}）"),

        // 57014 取消
        "57014" => format!("查询被取消（pg_cancel_backend）：{raw}"),

        // 3D000 数据库不存在
        "3D000" => format!("数据库不存在：{raw}"),

        // 0A000 不支持的特性
        "0A000" => format!("不支持的特性：{raw}"),

        // 22xxx 数据异常
        "22001" => format!("字段值过长（{raw}）"),
        "22003" => format!("数值越界（{raw}）"),
        "22007" => format!("时间格式无效（{raw}）"),
        "22P02" => format!("文本表示无效（类型转换失败）：{raw}"),
        "22023" => format!("参数值无效（{raw}）"),

        _ => raw.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_known_codes() {
        assert!(postgres_error_friendly("23505", "duplicate").contains("唯一键冲突"));
        assert!(postgres_error_friendly("42P01", "no table").contains("表/视图不存在"));
        assert!(postgres_error_friendly("28P01", "bad password").contains("用户名或密码"));
        assert!(postgres_error_friendly("57014", "canceled").contains("查询被取消"));
    }

    #[test]
    fn friendly_unknown_returns_raw() {
        assert_eq!(postgres_error_friendly("99999", "raw msg"), "raw msg");
    }
}
