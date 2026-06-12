//! 单元格转换：BSON 值 / Extended JSON 包装 → Cell（类型短标签 + 显示文本）。
//! 覆盖 Extended JSON v2 全部 18 种 BSON 类型，canonical / relaxed 两种形态。

use serde_json::Value;

/// 单元格视图：原始 BSON 类型 + 显示文本
#[derive(Debug, Clone)]
pub struct Cell {
    pub text: String,
    /// 类型短标签：text / int / double / bool / null / object / array / oid / decimal / date
    pub kind: &'static str,
}

/// 顶层字段值 → 单元格（第一层，不递归）：
/// 标量 / ExtJSON 包装 → 原值；嵌套对象 → "{N 字段}"；数组 → "[N 项]"。完整内容由下钻查看
pub(super) fn cell_for_value(v: &Value) -> Cell {
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
/// pub(super)：flatten / filter 复用同一套类型识别，避免重写 18 种 BSON 分支
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

/// 标量值 → Cell
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
