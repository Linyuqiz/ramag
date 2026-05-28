//! BSON ↔ serde_json::Value 双向转换。BSON 特殊类型走 Extended JSON 风格：
//! ObjectId → `{"$oid":"..."}`、Decimal128 → `{"$numberDecimal":"..."}`、DateTime → `{"$date":"ISO8601"}`

use bson::{Bson, Document};
use ramag_domain::error::{DomainError, Result};
use serde_json::Value;

/// BSON Bson → serde_json::Value（relaxed Extended JSON）。
/// relaxed 模式保留可读性（Int64/Double 直接输出数字），canonical 模式会给所有类型加包装
pub fn bson_to_json(b: Bson) -> Value {
    b.into_relaxed_extjson()
}

pub fn document_to_json(doc: Document) -> Value {
    Bson::Document(doc).into_relaxed_extjson()
}

/// serde_json::Value → BSON Bson。识别 Extended JSON 形态（$oid / $numberDecimal 等）。
/// 借 bson 的 serde::Deserialize impl，统一走 serde 反序列化
pub fn json_to_bson(v: Value) -> Result<Bson> {
    serde_json::from_value(v)
        .map_err(|e| DomainError::InvalidConfig(format!("JSON 解析 BSON 失败：{e}")))
}

/// 强制返回 Document（顶层必须是对象）。filter / update / sort / projection 等场景用
pub fn json_to_document(v: Value) -> Result<Document> {
    match json_to_bson(v)? {
        Bson::Document(d) => Ok(d),
        other => Err(DomainError::InvalidConfig(format!(
            "期望 JSON 对象，实际：{other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roundtrip_basic_doc() {
        let v = json!({"a": 1, "b": "hello", "c": [1, 2, 3]});
        let doc = json_to_document(v.clone()).unwrap();
        let back = document_to_json(doc);
        assert_eq!(back["a"], json!(1));
        assert_eq!(back["b"], json!("hello"));
        assert_eq!(back["c"], json!([1, 2, 3]));
    }

    #[test]
    fn objectid_extjson_roundtrip() {
        let v = json!({"$oid": "507f1f77bcf86cd799439011"});
        let bson = json_to_bson(v.clone()).unwrap();
        assert!(matches!(bson, Bson::ObjectId(_)));
        let back = bson_to_json(bson);
        // 走 extjson 后会保留 $oid 包装
        assert_eq!(back["$oid"], json!("507f1f77bcf86cd799439011"));
    }

    #[test]
    fn non_object_top_level_rejected() {
        let v = json!([1, 2, 3]);
        assert!(json_to_document(v).is_err());
    }
}
