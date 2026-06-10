//! 常用 MongoDB 命令示例模板（runCommand 风格 JSON），collection 名优先用当前 Tab 的

/// 生成（菜单标签, JSON 命令）列表。`collection` 为空时用占位名
pub(crate) fn mongo_examples(collection: &str) -> Vec<(&'static str, String)> {
    let c = if collection.trim().is_empty() {
        "your_collection"
    } else {
        collection
    };
    TEMPLATES
        .iter()
        .map(|(label, tpl)| (*label, tpl.replace("__COLL__", c)))
        .collect()
}

/// `__COLL__` 在生成时替换为实际 collection 名
const TEMPLATES: &[(&str, &str)] = &[
    (
        "查询 find",
        r#"{
  "find": "__COLL__",
  "filter": {},
  "sort": { "_id": -1 },
  "limit": 100
}"#,
    ),
    (
        "聚合 aggregate",
        r#"{
  "aggregate": "__COLL__",
  "pipeline": [
    { "$match": {} },
    { "$group": { "_id": "$your_field", "count": { "$sum": 1 } } }
  ],
  "cursor": {}
}"#,
    ),
    (
        "计数 count",
        r#"{
  "count": "__COLL__",
  "query": {}
}"#,
    ),
    (
        "去重 distinct",
        r#"{
  "distinct": "__COLL__",
  "key": "your_field",
  "query": {}
}"#,
    ),
    (
        "插入 insert",
        r#"{
  "insert": "__COLL__",
  "documents": [
    { "name": "demo" }
  ]
}"#,
    ),
    (
        "更新 update",
        r#"{
  "update": "__COLL__",
  "updates": [
    { "q": { "name": "demo" }, "u": { "$set": { "name": "new" } }, "multi": false }
  ]
}"#,
    ),
    (
        "删除 delete",
        r#"{
  "delete": "__COLL__",
  "deletes": [
    { "q": { "name": "demo" }, "limit": 1 }
  ]
}"#,
    ),
    (
        "查看索引 listIndexes",
        r#"{
  "listIndexes": "__COLL__"
}"#,
    ),
    (
        "新建索引 createIndexes",
        r#"{
  "createIndexes": "__COLL__",
  "indexes": [
    { "key": { "your_field": 1 }, "name": "your_field_1" }
  ]
}"#,
    ),
    (
        "集合统计 collStats",
        r#"{
  "collStats": "__COLL__"
}"#,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_are_valid_json_with_collection() {
        for (label, cmd) in mongo_examples("users") {
            let parsed: Result<serde_json::Value, _> = serde_json::from_str(&cmd);
            assert!(parsed.is_ok(), "示例 {label} 不是合法 JSON: {cmd}");
            assert!(cmd.contains("users"), "示例 {label} 未替换 collection 名");
        }
    }

    #[test]
    fn empty_collection_falls_back_to_placeholder() {
        let items = mongo_examples("");
        assert!(items.iter().all(|(_, cmd)| cmd.contains("your_collection")));
    }
}
