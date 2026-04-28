//! 结果集导出工具
//!
//! 把 [`QueryResult`] 序列化为 CSV / JSON 文本，供 UI 层写文件 / 复制剪贴板使用。

use ramag_domain::entities::{QueryResult, Value};

/// 把结果集导出为 CSV 字符串
///
/// - 第一行：列名（含逗号/引号会自动转义）
/// - 后续行：值（NULL = 空字段；BLOB = base64-like hex）
pub fn to_csv(result: &QueryResult) -> String {
    let mut out = String::with_capacity(result.rows.len() * 64);

    // 表头
    let header: Vec<String> = result.columns.iter().map(|c| csv_escape(c)).collect();
    out.push_str(&header.join(","));
    out.push('\n');

    // 数据行
    for row in &result.rows {
        let cells: Vec<String> = row.values.iter().map(value_to_csv_field).collect();
        out.push_str(&cells.join(","));
        out.push('\n');
    }

    out
}

/// 把结果集导出为 JSON 字符串（pretty-printed 数组）
///
/// 每行 = 一个对象：`{ "col_name": value, ... }`
pub fn to_json(result: &QueryResult) -> String {
    use serde_json::{Map, Value as JsonValue};

    let mut arr: Vec<JsonValue> = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        let mut obj = Map::with_capacity(result.columns.len());
        for (i, col) in result.columns.iter().enumerate() {
            let v = row.values.get(i).cloned().unwrap_or(Value::Null);
            obj.insert(col.clone(), value_to_json(v));
        }
        arr.push(JsonValue::Object(obj));
    }
    serde_json::to_string_pretty(&JsonValue::Array(arr)).unwrap_or_else(|_| "[]".to_string())
}

/// 把结果集导出为 GitHub-flavored Markdown 表格
///
/// `| col1 | col2 |` + `| --- | --- |` + N 行
/// 单元格转义：管道 `|` → `\|`、反斜杠 `\` → `\\`、换行/回车 → 空格
pub fn to_markdown(result: &QueryResult) -> String {
    let escape = |s: &str| -> String {
        s.replace('\\', "\\\\")
            .replace('|', "\\|")
            .replace('\n', " ")
            .replace('\r', "")
    };
    let header = result
        .columns
        .iter()
        .map(|c| escape(c))
        .collect::<Vec<_>>()
        .join(" | ");
    let sep = result
        .columns
        .iter()
        .map(|_| "---")
        .collect::<Vec<_>>()
        .join(" | ");
    let mut lines = Vec::with_capacity(2 + result.rows.len());
    lines.push(format!("| {header} |"));
    lines.push(format!("| {sep} |"));
    for row in &result.rows {
        let body = row
            .values
            .iter()
            .map(|v| escape(&v.display_preview(usize::MAX)))
            .collect::<Vec<_>>()
            .join(" | ");
        lines.push(format!("| {body} |"));
    }
    lines.join("\n")
}

/// CSV 字段转义：包含逗号 / 引号 / 换行时用双引号包裹，内部引号 `"` → `""`
fn csv_escape(s: &str) -> String {
    let needs_quote = s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r');
    if !needs_quote {
        return s.to_string();
    }
    let escaped = s.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

fn value_to_csv_field(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Text(s) => csv_escape(s),
        Value::Bytes(b) => csv_escape(&hex::encode(b)),
        Value::DateTime(dt) => csv_escape(&dt.to_rfc3339()),
        Value::Json(j) => csv_escape(&j.to_string()),
    }
}

fn value_to_json(v: Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Value::Null => J::Null,
        Value::Bool(b) => J::Bool(b),
        Value::Int(i) => J::Number(i.into()),
        Value::Float(f) => serde_json::Number::from_f64(f)
            .map(J::Number)
            .unwrap_or(J::Null),
        Value::Text(s) => J::String(s),
        Value::Bytes(b) => J::String(hex::encode(b)),
        Value::DateTime(dt) => J::String(dt.to_rfc3339()),
        Value::Json(j) => j,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ramag_domain::entities::Row;

    fn sample_result() -> QueryResult {
        QueryResult {
            columns: vec!["id".into(), "name".into(), "data".into()],
            column_types: Vec::new(),
            rows: vec![
                Row {
                    values: vec![Value::Int(1), Value::Text("张三".into()), Value::Null],
                },
                Row {
                    values: vec![
                        Value::Int(2),
                        Value::Text("李, 四".into()),      // 含逗号
                        Value::Text("\"escaped\"".into()), // 含引号
                    ],
                },
            ],
            affected_rows: 0,
            elapsed_ms: 5,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn csv_basic() {
        let csv = to_csv(&sample_result());
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines[0], "id,name,data");
        assert_eq!(lines[1], "1,张三,");
        // 含逗号的字段应被引号包裹
        assert!(lines[2].contains("\"李, 四\""));
        // 引号要转义为 ""
        assert!(lines[2].contains("\"\"\"escaped\"\"\""));
    }

    #[test]
    fn json_basic() {
        let json = to_json(&sample_result());
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["id"], 1);
        assert_eq!(arr[0]["name"], "张三");
        assert!(arr[0]["data"].is_null());
        assert_eq!(arr[1]["name"], "李, 四");
    }
}
