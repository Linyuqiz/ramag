//! 文档扁平化：只解析第一层字段（列 = 顶层 key）。嵌套对象/数组出摘要，完整内容靠表格双击查看；
//! Extended JSON 包装类型（$oid / $numberDecimal / $date / $binary）取内部值
//!
//! 例：
//!   `{"specs":{"cpu":"i7"}}` → `{"specs": "{1 字段}"}`（不再展开成 specs.cpu）
//!   `{"tags":["x","y"]}` → `{"tags": "[2 项]"}`
//!   `{"_id":{"$oid":"abc..."}}` → `{"_id": "abc..."}`

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::Value;

/// 单元格视图：原始 BSON 类型 + 显示文本
#[derive(Debug, Clone)]
pub struct Cell {
    pub text: String,
    /// 类型短标签：text / int / double / bool / null / object / array / oid / decimal / date
    pub kind: &'static str,
}

/// 单列元信息
#[derive(Debug, Clone)]
pub struct Column {
    /// dotted path
    pub path: String,
    /// 类型：取该列下首个非 null 的 kind
    pub kind: &'static str,
}

/// 扁平化的表格
#[derive(Debug, Clone, Default)]
pub struct FlatTable {
    pub columns: Vec<Column>,
    /// 每行 = 列对齐的 cell 字符串（缺字段填空字符串，kind=null）
    pub rows: Vec<Vec<Cell>>,
}

impl FlatTable {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// 把文档列表扁平化成表格
pub fn build_flat_table(docs: &[Value]) -> FlatTable {
    // 1) 扁平化每条文档
    let flat_rows: Vec<BTreeMap<String, Cell>> = docs.iter().map(flatten_doc).collect();

    // 2) 列发现 + 类型推断
    let mut col_seen: HashSet<String> = HashSet::new();
    let mut col_order: Vec<String> = Vec::new();
    let mut col_kinds: HashMap<String, &'static str> = HashMap::new();
    for row in &flat_rows {
        for (k, v) in row {
            if col_seen.insert(k.clone()) {
                col_order.push(k.clone());
            }
            // 取首个非 null kind 作为该列类型
            col_kinds.entry(k.clone()).or_insert(v.kind);
            if let Some(existing) = col_kinds.get_mut(k)
                && *existing == "null"
                && v.kind != "null"
            {
                *existing = v.kind;
            }
        }
    }

    // 3) 排序：_id 优先；其它按插入顺序
    col_order.sort_by(|a, b| match (a.as_str(), b.as_str()) {
        ("_id", _) => std::cmp::Ordering::Less,
        (_, "_id") => std::cmp::Ordering::Greater,
        _ => std::cmp::Ordering::Equal,
    });

    let columns: Vec<Column> = col_order
        .iter()
        .map(|p| Column {
            path: p.clone(),
            kind: col_kinds.get(p).copied().unwrap_or("null"),
        })
        .collect();

    // 4) 行 → 列对齐
    let empty_cell = Cell {
        text: String::new(),
        kind: "null",
    };
    let rows: Vec<Vec<Cell>> = flat_rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|c| {
                    row.get(&c.path)
                        .cloned()
                        .unwrap_or_else(|| empty_cell.clone())
                })
                .collect()
        })
        .collect();

    FlatTable { columns, rows }
}

/// 扁平化单文档：只解析第一层字段（列 = 顶层 key），嵌套不再递归成 dotted-path
fn flatten_doc(v: &Value) -> BTreeMap<String, Cell> {
    let mut out = BTreeMap::new();
    if let Value::Object(map) = v {
        for (k, vv) in map {
            out.insert(k.clone(), cell_for_value(vv));
        }
    } else {
        out.insert("_value".to_string(), cell_for_value(v));
    }
    out
}

/// 顶层字段值 → 单元格（第一层，不递归）：
/// 标量 / ExtJSON 包装 → 原值；嵌套对象 → "{N 字段}"；数组 → "[N 项]"。完整内容由下钻查看
fn cell_for_value(v: &Value) -> Cell {
    match v {
        // ExtJSON 包装（$oid/$date…）取内部值；普通对象出字段数摘要
        Value::Object(map) => extjson_cell(map).unwrap_or_else(|| Cell {
            text: format!("{{{} 字段}}", map.len()),
            kind: "object",
        }),
        Value::Array(arr) => Cell {
            text: format!("[{} 项]", arr.len()),
            kind: "array",
        },
        _ => scalar_to_cell(v).unwrap_or_else(|| Cell {
            text: String::new(),
            kind: "null",
        }),
    }
}

/// 识别 Extended JSON 包装对象，返回内部值。
/// 覆盖 MongoDB Extended JSON v2 全部 18 种 BSON 类型 + canonical/relaxed 两种形态。
/// pub(super)：树视图（tree.rs）复用同一套类型识别，避免重写 18 种 BSON 分支
pub(super) fn extjson_cell(map: &serde_json::Map<String, Value>) -> Option<Cell> {
    // ObjectId
    if let Some(v) = map.get("$oid").and_then(|x| x.as_str()) {
        return Some(Cell {
            text: v.to_string(),
            kind: "oid",
        });
    }
    // Decimal128
    if let Some(v) = map.get("$numberDecimal").and_then(|x| x.as_str()) {
        return Some(Cell {
            text: v.to_string(),
            kind: "decimal",
        });
    }
    // DateTime：兼容 relaxed `{$date: "ISO"}` 与 canonical `{$date: {$numberLong: "ms"}}`
    if let Some(d) = map.get("$date") {
        let text = match d {
            Value::String(s) => s.clone(),
            Value::Object(inner) => inner
                .get("$numberLong")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| d.to_string()),
            _ => d.to_string(),
        };
        return Some(Cell { text, kind: "date" });
    }
    // Binary：canonical `{$binary: {base64, subType}}`
    if let Some(b) = map.get("$binary").and_then(|x| x.as_object())
        && let Some(b64) = b.get("base64").and_then(|x| x.as_str())
    {
        let sub = b.get("subType").and_then(|x| x.as_str()).unwrap_or("00");
        return Some(Cell {
            text: format!("[binary {} b64chars, subType={sub}]", b64.len()),
            kind: "binary",
        });
    }
    // Int64
    if let Some(v) = map.get("$numberLong").and_then(|x| x.as_str()) {
        return Some(Cell {
            text: v.to_string(),
            kind: "int",
        });
    }
    // Int32 canonical
    if let Some(v) = map.get("$numberInt").and_then(|x| x.as_str()) {
        return Some(Cell {
            text: v.to_string(),
            kind: "int",
        });
    }
    // Double canonical（含 Infinity / -Infinity / NaN 字面量）
    if let Some(v) = map.get("$numberDouble").and_then(|x| x.as_str()) {
        return Some(Cell {
            text: v.to_string(),
            kind: "double",
        });
    }
    // Regex：{$regularExpression: {pattern, options}}
    if let Some(re) = map.get("$regularExpression").and_then(|x| x.as_object()) {
        let pattern = re.get("pattern").and_then(|x| x.as_str()).unwrap_or("");
        let options = re.get("options").and_then(|x| x.as_str()).unwrap_or("");
        return Some(Cell {
            text: format!("/{pattern}/{options}"),
            kind: "regex",
        });
    }
    // Timestamp：{$timestamp: {t, i}}，多用于 oplog / replication
    if let Some(ts) = map.get("$timestamp").and_then(|x| x.as_object()) {
        let t = ts.get("t").and_then(|x| x.as_u64()).unwrap_or(0);
        let i = ts.get("i").and_then(|x| x.as_u64()).unwrap_or(0);
        return Some(Cell {
            text: format!("Timestamp({t}, {i})"),
            kind: "ts",
        });
    }
    // MinKey / MaxKey
    if map.contains_key("$minKey") {
        return Some(Cell {
            text: "MinKey".to_string(),
            kind: "minkey",
        });
    }
    if map.contains_key("$maxKey") {
        return Some(Cell {
            text: "MaxKey".to_string(),
            kind: "maxkey",
        });
    }
    // Undefined（deprecated；老库迁移可能仍存在）
    if map.contains_key("$undefined") {
        return Some(Cell {
            text: "undefined".to_string(),
            kind: "undef",
        });
    }
    // JavaScript Code（可选 $scope = CodeWithScope，统一只显主体）
    if let Some(code) = map.get("$code").and_then(|x| x.as_str()) {
        return Some(Cell {
            text: code.to_string(),
            kind: "code",
        });
    }
    // Symbol（deprecated）
    if let Some(s) = map.get("$symbol").and_then(|x| x.as_str()) {
        return Some(Cell {
            text: s.to_string(),
            kind: "symbol",
        });
    }
    // DBPointer（deprecated）
    if map.contains_key("$dbPointer") {
        return Some(Cell {
            text: serde_json::to_string(&map["$dbPointer"]).unwrap_or_default(),
            kind: "dbptr",
        });
    }
    None
}

/// 标量值 → Cell；pub(super) 供 tree.rs 复用
pub(super) fn scalar_to_cell(v: &Value) -> Option<Cell> {
    match v {
        Value::Null => Some(Cell {
            text: String::new(),
            kind: "null",
        }),
        Value::Bool(b) => Some(Cell {
            text: b.to_string(),
            kind: "bool",
        }),
        Value::Number(n) => {
            let kind = if n.is_i64() || n.is_u64() {
                "int"
            } else {
                "double"
            };
            Some(Cell {
                text: n.to_string(),
                kind,
            })
        }
        Value::String(s) => Some(Cell {
            text: s.clone(),
            kind: "text",
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flatten_scalar_object() {
        let t = build_flat_table(&[json!({"a": 1, "b": "x"})]);
        assert_eq!(t.columns.len(), 2);
        let a = t.columns.iter().find(|c| c.path == "a").unwrap();
        assert_eq!(a.kind, "int");
    }

    #[test]
    fn flatten_nested_object_is_summary() {
        let t = build_flat_table(&[json!({"specs": {"cpu": "i7", "ram": 16}})]);
        // 只解析第一层：specs 是一列摘要，不展开成 specs.cpu / specs.ram
        assert_eq!(t.columns.len(), 1);
        assert_eq!(t.columns[0].path, "specs");
        assert_eq!(t.columns[0].kind, "object");
        assert_eq!(t.rows[0][0].text, "{2 字段}");
    }

    #[test]
    fn flatten_oid_unwrapped() {
        let t = build_flat_table(&[json!({"_id": {"$oid": "507f1f77bcf86cd799439011"}})]);
        let cell = &t.rows[0][0];
        assert_eq!(cell.kind, "oid");
        assert_eq!(cell.text, "507f1f77bcf86cd799439011");
    }

    #[test]
    fn flatten_decimal_unwrapped() {
        let t = build_flat_table(&[json!({"price": {"$numberDecimal": "1299.99"}})]);
        assert_eq!(t.rows[0][0].kind, "decimal");
        assert_eq!(t.rows[0][0].text, "1299.99");
    }

    #[test]
    fn flatten_array_is_summary() {
        let t = build_flat_table(&[json!({"tags": ["a", "b"]})]);
        assert_eq!(t.rows[0][0].kind, "array");
        assert_eq!(t.rows[0][0].text, "[2 项]");
    }

    #[test]
    fn flatten_columns_id_first() {
        let t = build_flat_table(&[json!({"name": "a", "_id": "x"})]);
        assert_eq!(t.columns[0].path, "_id");
    }

    #[test]
    fn flatten_missing_field_filled_null() {
        let t = build_flat_table(&[json!({"a": 1}), json!({"b": 2})]);
        assert_eq!(t.columns.len(), 2);
        assert_eq!(t.rows[0].len(), 2);
        assert_eq!(t.rows[1].len(), 2);
    }

    #[test]
    fn flatten_date_canonical_form() {
        // canonical {$date: {$numberLong: "ms"}}
        let t = build_flat_table(&[json!({"ts": {"$date": {"$numberLong": "1700000000000"}}})]);
        assert_eq!(t.rows[0][0].kind, "date");
        assert_eq!(t.rows[0][0].text, "1700000000000");
    }

    #[test]
    fn flatten_timestamp() {
        let t = build_flat_table(&[json!({"ts": {"$timestamp": {"t": 1700, "i": 5}}})]);
        assert_eq!(t.rows[0][0].kind, "ts");
        assert!(t.rows[0][0].text.contains("1700"));
    }

    #[test]
    fn flatten_regex() {
        let t = build_flat_table(&[json!({
            "rx": {"$regularExpression": {"pattern": "^abc", "options": "i"}}
        })]);
        assert_eq!(t.rows[0][0].kind, "regex");
        assert_eq!(t.rows[0][0].text, "/^abc/i");
    }

    #[test]
    fn flatten_minkey_maxkey() {
        let t = build_flat_table(&[json!({"lo": {"$minKey": 1}, "hi": {"$maxKey": 1}})]);
        let lo = t
            .columns
            .iter()
            .position(|c| c.path == "lo")
            .map(|i| &t.rows[0][i])
            .unwrap();
        let hi = t
            .columns
            .iter()
            .position(|c| c.path == "hi")
            .map(|i| &t.rows[0][i])
            .unwrap();
        assert_eq!(lo.kind, "minkey");
        assert_eq!(hi.kind, "maxkey");
    }

    #[test]
    fn flatten_undefined() {
        let t = build_flat_table(&[json!({"x": {"$undefined": true}})]);
        assert_eq!(t.rows[0][0].kind, "undef");
        assert_eq!(t.rows[0][0].text, "undefined");
    }

    #[test]
    fn flatten_code_and_symbol() {
        let t = build_flat_table(&[json!({
            "fn": {"$code": "function(){}"},
            "sym": {"$symbol": "alpha"}
        })]);
        let f = t
            .columns
            .iter()
            .position(|c| c.path == "fn")
            .map(|i| &t.rows[0][i])
            .unwrap();
        let s = t
            .columns
            .iter()
            .position(|c| c.path == "sym")
            .map(|i| &t.rows[0][i])
            .unwrap();
        assert_eq!(f.kind, "code");
        assert_eq!(s.kind, "symbol");
    }

    #[test]
    fn flatten_int32_canonical() {
        let t = build_flat_table(&[json!({"n": {"$numberInt": "42"}})]);
        assert_eq!(t.rows[0][0].kind, "int");
        assert_eq!(t.rows[0][0].text, "42");
    }

    #[test]
    fn flatten_double_canonical() {
        let t = build_flat_table(&[json!({"d": {"$numberDouble": "Infinity"}})]);
        assert_eq!(t.rows[0][0].kind, "double");
        assert_eq!(t.rows[0][0].text, "Infinity");
    }

    #[test]
    fn flatten_binary_with_subtype() {
        let t = build_flat_table(&[json!({
            "blob": {"$binary": {"base64": "aGVsbG8=", "subType": "00"}}
        })]);
        assert_eq!(t.rows[0][0].kind, "binary");
        assert!(t.rows[0][0].text.contains("subType=00"));
    }
}
