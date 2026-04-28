//! 查询与结果集实体

use serde::{Deserialize, Serialize};

/// 一次 SQL 查询请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Query {
    /// 原始 SQL 文本
    pub sql: String,
    /// 是否只允许只读语句（用于安全模式）
    pub read_only: bool,
    /// 当前会话默认库（执行前 driver 在同一连接上发 USE 切换）
    /// 仅当连接配置未指定 database 且 SQL 写裸表名时必需，否则可为 None
    #[serde(default)]
    pub default_schema: Option<String>,
    /// 自动 LIMIT 注入：Some(n) 代表对未带 LIMIT 的最外层 SELECT/WITH 自动追加 ` LIMIT n`
    /// None 表示完全不注入（用户在 UI 上关闭，或写 `-- ramag:no-limit` 注释）
    /// 默认 None：driver 不会主动改写 SQL，避免破坏一致性；UI 默认值由调用层决定
    #[serde(default)]
    pub auto_limit: Option<u32>,
}

impl Query {
    pub fn new(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            read_only: false,
            default_schema: None,
            auto_limit: None,
        }
    }

    pub fn read_only(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            read_only: true,
            default_schema: None,
            auto_limit: None,
        }
    }

    /// 链式设置默认库
    pub fn with_schema(mut self, schema: impl Into<String>) -> Self {
        self.default_schema = Some(schema.into());
        self
    }

    /// 链式设置自动 LIMIT 注入数量；调用方传 None 等价于不注入
    pub fn with_auto_limit(mut self, limit: Option<u32>) -> Self {
        self.auto_limit = limit;
        self
    }
}

/// 查询结果（统一抽象，跨数据库类型）
///
/// 即使是 INSERT/UPDATE 也包装成 QueryResult（rows 为空，affected_rows 有值）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    /// 列名
    pub columns: Vec<String>,
    /// 列类型名（与 columns 一一对应；driver 不提供时为空 Vec）
    /// 仅用于 UI 表头展示，导出/补全等不读取
    #[serde(default)]
    pub column_types: Vec<String>,
    /// 数据行
    pub rows: Vec<Row>,
    /// 受影响行数（INSERT/UPDATE/DELETE）
    pub affected_rows: u64,
    /// 执行耗时（毫秒）
    pub elapsed_ms: u64,
    /// 服务端警告（MySQL 来自 SHOW WARNINGS）；空 Vec 表示无警告
    /// 多语句执行时累积每条 statement 的 warnings
    #[serde(default)]
    pub warnings: Vec<Warning>,
}

/// 服务端警告（MySQL 的 SHOW WARNINGS 输出对应的一行）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Warning {
    /// 级别："Note" / "Warning" / "Error"（MySQL 一般大写首字母）
    pub level: String,
    /// 错误码（对应 mysql_errno()，如 1265 / 1366）
    pub code: u32,
    /// 中文/英文消息原文
    pub message: String,
}

/// 一行数据（按列顺序排列）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    pub values: Vec<Value>,
}

/// 单元格值
///
/// 用 enum 容纳不同数据库类型的所有可能值，UI 层根据 variant 渲染
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    /// 时间戳，UTC，纳秒精度
    DateTime(chrono::DateTime<chrono::Utc>),
    /// JSON 嵌套结构（MySQL 5.7+ JSON 列、PG jsonb）
    Json(serde_json::Value),
}

impl Value {
    /// 用于 UI 显示的简短预览（截断长字符串）
    pub fn display_preview(&self, max_len: usize) -> String {
        match self {
            Value::Null => "NULL".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Text(s) => truncate(s, max_len),
            Value::Bytes(b) => format!("[{} bytes]", b.len()),
            Value::DateTime(dt) => dt.to_rfc3339(),
            Value::Json(v) => truncate(&v.to_string(), max_len),
        }
    }

    /// 用于嵌入 SQL 字面量的形式（INSERT 语句生成）
    ///
    /// - Null → `NULL`
    /// - Bool → `TRUE` / `FALSE`
    /// - Int/Float → 数字字面量
    /// - Text/Json → `'escaped'`（单引号转义 → `''`，反斜杠 → `\\`）
    /// - Bytes → `0xHEX`（MySQL 风格十六进制字面量）
    /// - DateTime → `'YYYY-MM-DD HH:MM:SS'`（MySQL DATETIME 默认格式，UTC）
    pub fn to_sql_literal(&self) -> String {
        match self {
            Value::Null => "NULL".to_string(),
            Value::Bool(b) => {
                if *b {
                    "TRUE".to_string()
                } else {
                    "FALSE".to_string()
                }
            }
            Value::Int(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Text(s) => format!("'{}'", escape_sql_string(s)),
            Value::Bytes(b) => {
                let mut hex = String::with_capacity(2 + b.len() * 2);
                hex.push_str("0x");
                for byte in b {
                    hex.push_str(&format!("{:02x}", byte));
                }
                hex
            }
            Value::DateTime(dt) => {
                format!("'{}'", dt.format("%Y-%m-%d %H:%M:%S"))
            }
            Value::Json(v) => format!("'{}'", escape_sql_string(&v.to_string())),
        }
    }

    /// 单元格编辑弹框初值：JSON 走 pretty 多行，其它与 clipboard 一致
    ///
    /// 让用户在编辑框里看到结构化的 JSON（每个键独立一行 + 缩进），
    /// 提交回数据库前 sqlx 会自动归一，无需调用方手动 minify
    pub fn display_for_edit(&self) -> String {
        match self {
            Value::Json(v) => serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()),
            other => other.to_clipboard_string(),
        }
    }

    /// 复制到剪贴板时的完整字符串（不截断，无修饰）
    ///
    /// - Null 复制为空串（更适合粘贴到表单/SQL）
    /// - Bytes 转为连续小写 hex
    /// - DateTime 用 RFC3339（含时区，可往返）
    /// - Json 用最小化形式（无格式化空格）
    pub fn to_clipboard_string(&self) -> String {
        match self {
            Value::Null => String::new(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Text(s) => s.clone(),
            Value::Bytes(b) => {
                let mut out = String::with_capacity(b.len() * 2);
                for byte in b {
                    out.push_str(&format!("{:02x}", byte));
                }
                out
            }
            Value::DateTime(dt) => dt.to_rfc3339(),
            Value::Json(v) => v.to_string(),
        }
    }
}

/// SQL 字符串字面量转义：反斜杠 + 单引号
/// 用于 to_sql_literal 中嵌入 'xxx' 形式
fn escape_sql_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("''"),
            _ => out.push(ch),
        }
    }
    out
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len).collect();
        format!("{}…", truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn clipboard_null_is_empty() {
        assert_eq!(Value::Null.to_clipboard_string(), "");
    }

    #[test]
    fn clipboard_primitive() {
        // 浮点用非 π 近似（避开 clippy::approx_constant）
        assert_eq!(Value::Bool(true).to_clipboard_string(), "true");
        assert_eq!(Value::Int(-42).to_clipboard_string(), "-42");
        assert_eq!(Value::Float(2.5).to_clipboard_string(), "2.5");
    }

    #[test]
    fn clipboard_text_not_truncated() {
        // 200 字应该完整保留，不带省略号
        let long: String = "字".repeat(200);
        assert_eq!(Value::Text(long.clone()).to_clipboard_string(), long);
    }

    #[test]
    fn clipboard_bytes_hex() {
        let v = Value::Bytes(vec![0x00, 0xAB, 0xff]);
        assert_eq!(v.to_clipboard_string(), "00abff");
    }

    #[test]
    fn clipboard_datetime_rfc3339() {
        let dt = chrono::Utc
            .with_ymd_and_hms(2026, 4, 26, 17, 30, 0)
            .unwrap();
        let s = Value::DateTime(dt).to_clipboard_string();
        assert!(s.starts_with("2026-04-26T17:30:00"));
    }

    #[test]
    fn sql_literal_basic() {
        assert_eq!(Value::Null.to_sql_literal(), "NULL");
        assert_eq!(Value::Bool(true).to_sql_literal(), "TRUE");
        assert_eq!(Value::Bool(false).to_sql_literal(), "FALSE");
        assert_eq!(Value::Int(42).to_sql_literal(), "42");
    }

    #[test]
    fn sql_literal_text_escapes_quote() {
        assert_eq!(
            Value::Text("O'Reilly".to_string()).to_sql_literal(),
            "'O''Reilly'"
        );
        assert_eq!(Value::Text("a\\b".to_string()).to_sql_literal(), "'a\\\\b'");
    }

    #[test]
    fn sql_literal_bytes_hex() {
        assert_eq!(
            Value::Bytes(vec![0x00, 0xab, 0xff]).to_sql_literal(),
            "0x00abff"
        );
    }

    #[test]
    fn sql_literal_datetime_mysql_format() {
        let dt = chrono::Utc
            .with_ymd_and_hms(2026, 4, 8, 17, 31, 15)
            .unwrap();
        assert_eq!(
            Value::DateTime(dt).to_sql_literal(),
            "'2026-04-08 17:31:15'"
        );
    }

    #[test]
    fn clipboard_json_minified() {
        let v = Value::Json(serde_json::json!({"a": 1, "b": [2, 3]}));
        let s = v.to_clipboard_string();
        // 不含格式化空格
        assert!(!s.contains("\n"));
        assert!(s.contains("\"a\":1"));
    }
}
