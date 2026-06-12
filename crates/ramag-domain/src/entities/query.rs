//! 查询与结果集实体

use serde::{Deserialize, Serialize};

/// 一次 SQL 查询请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Query {
    pub sql: String,
    /// 会话默认库，driver 执行前发 USE 切换
    #[serde(default)]
    pub default_schema: Option<String>,
    /// 自动 LIMIT 注入：Some(n) 给未带 LIMIT 的最外层 SELECT/WITH 追加 `LIMIT n`；None 不注入
    #[serde(default)]
    pub auto_limit: Option<u32>,
}

impl Query {
    pub fn new(sql: impl Into<String>) -> Self {
        Self {
            sql: sql.into(),
            default_schema: None,
            auto_limit: None,
        }
    }

    pub fn with_schema(mut self, schema: impl Into<String>) -> Self {
        self.default_schema = Some(schema.into());
        self
    }

    pub fn with_auto_limit(mut self, limit: Option<u32>) -> Self {
        self.auto_limit = limit;
        self
    }
}

/// 查询结果（INSERT/UPDATE 也走这个，rows 空、affected_rows 有值）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    /// 列类型名，与 columns 一一对应；driver 不提供时为空。仅 UI 表头展示
    #[serde(default)]
    pub column_types: Vec<String>,
    pub rows: Vec<Row>,
    /// INSERT/UPDATE/DELETE 受影响行数
    pub affected_rows: u64,
    pub elapsed_ms: u64,
    /// MySQL SHOW WARNINGS；多语句执行时累积所有 statement 的警告
    #[serde(default)]
    pub warnings: Vec<Warning>,
}

/// 服务端警告（MySQL SHOW WARNINGS 一行）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Warning {
    /// "Note" / "Warning" / "Error"
    pub level: String,
    /// 对应 mysql_errno()
    pub code: u32,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    pub values: Vec<Value>,
}

/// 单元格值。UI 按 variant 选渲染方式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    /// UTC 纳秒精度
    DateTime(chrono::DateTime<chrono::Utc>),
    /// MySQL JSON 列、PG jsonb
    Json(serde_json::Value),
}

impl Value {
    /// UI 显示用的短预览（截断长字符串）
    pub fn display_preview(&self, max_len: usize) -> String {
        match self {
            Value::Null => "NULL".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Text(s) => sanitize_inline(&truncate(s, max_len)),
            Value::Bytes(b) => format!("[{} bytes]", b.len()),
            Value::DateTime(dt) => dt.to_rfc3339(),
            Value::Json(v) => truncate(&v.to_string(), max_len),
        }
    }

    /// 转 SQL 字面量。Bytes 走 MySQL 风格 `0xHEX`，DateTime 走 `'YYYY-MM-DD HH:MM:SS'`
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

    /// 单元格编辑初值：JSON 走 pretty 多行，其余等价 clipboard 形式
    pub fn display_for_edit(&self) -> String {
        match self {
            Value::Json(v) => serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()),
            other => other.to_clipboard_string(),
        }
    }

    /// 剪贴板字符串（完整，不截断）。Null→空串、Bytes→hex、DateTime→RFC3339、Json→紧凑
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

/// 单行预览清洗：换行符（\n / \r）替换为空格。
/// GPUI 单行文本 shaping 断言不允许 \n（含 \n 直接 panic→abort）；仅用于显示预览，
/// 不影响 to_clipboard_string / display_for_edit 等完整取值。无换行时零拷贝
fn sanitize_inline(s: &str) -> String {
    if s.contains(['\n', '\r']) {
        s.replace(['\n', '\r'], " ")
    } else {
        s.to_string()
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
        assert_eq!(Value::Bool(true).to_clipboard_string(), "true");
        assert_eq!(Value::Int(-42).to_clipboard_string(), "-42");
        assert_eq!(Value::Float(2.5).to_clipboard_string(), "2.5");
    }

    #[test]
    fn clipboard_text_not_truncated() {
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
    fn preview_text_strips_newlines() {
        // 含换行的文本预览必须压成单行，否则结果表格渲染 panic
        let v = Value::Text("line1\nline2\r\nline3".to_string());
        let p = v.display_preview(80);
        assert!(!p.contains('\n') && !p.contains('\r'));
    }

    #[test]
    fn clipboard_json_minified() {
        let v = Value::Json(serde_json::json!({"a": 1, "b": [2, 3]}));
        let s = v.to_clipboard_string();
        assert!(!s.contains("\n"));
        assert!(s.contains("\"a\":1"));
    }
}
