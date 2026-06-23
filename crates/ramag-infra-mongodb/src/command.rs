//! MongoDB 写命令识别：生产模式只读保护用。
//! 命令是 runCommand 风格 JSON（顶层第一个 key 即命令名）。为避开 serde_json 无 preserve_order
//! 时 Object 的 key 顺序问题，这里遍历全部顶层 key 匹配写命令名（命令参数是 value，不会误判）。
//! 特例：`aggregate` 本身只读，但 pipeline 含 `$out` / `$merge` 会写出集合，需单独识别

use serde_json::Value;

/// runCommand 文档是否为写命令
pub fn command_is_write(command: &Value) -> bool {
    let Some(obj) = command.as_object() else {
        return false;
    };
    // 顶层任一 key 命中写命令名即写（大小写不敏感）
    if obj
        .keys()
        .any(|k| WRITE_COMMANDS.contains(&k.to_ascii_lowercase().as_str()))
    {
        return true;
    }
    // aggregate 只读，但 pipeline 带 $out / $merge 会写
    let has_aggregate = obj.keys().any(|k| k.eq_ignore_ascii_case("aggregate"));
    if has_aggregate
        && let Some(pipeline) = obj.get("pipeline").and_then(|v| v.as_array())
        && pipeline_has_write_stage(pipeline)
    {
        return true;
    }
    false
}

/// 聚合管线是否含写出阶段（`$out` / `$merge`）
pub fn pipeline_has_write_stage(pipeline: &[Value]) -> bool {
    pipeline.iter().any(|stage| {
        stage.as_object().is_some_and(|o| {
            o.keys()
                .any(|k| k.eq_ignore_ascii_case("$out") || k.eq_ignore_ascii_case("$merge"))
        })
    })
}

/// 写命令名（小写）。覆盖文档写 / DDL / 索引 / 用户角色 / 物理维护等
const WRITE_COMMANDS: &[&str] = &[
    // 文档写
    "insert",
    "update",
    "delete",
    "findandmodify",
    "bulkwrite",
    // 集合 / 库 DDL
    "drop",
    "dropdatabase",
    "create",
    "renamecollection",
    "collmod",
    "mapreduce",
    "emptycapped",
    "converttocapped",
    "clonecollectionascapped",
    // 索引
    "createindexes",
    "dropindexes",
    "reindex",
    "compact",
    // 用户 / 角色
    "createuser",
    "dropuser",
    "updateuser",
    "grantrolestouser",
    "revokerolesfromuser",
    "createrole",
    "droprole",
    "updaterole",
    "grantprivilegestorole",
    "revokeprivilegesfromrole",
    "dropallusersfromdatabase",
    "dropallrolesfromdatabase",
    // 集群 / 兼容性
    "setfeaturecompatibilityversion",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn write_commands_detected() {
        assert!(command_is_write(&json!({"insert": "c", "documents": []})));
        assert!(command_is_write(&json!({"update": "c", "updates": []})));
        assert!(command_is_write(&json!({"delete": "c", "deletes": []})));
        assert!(command_is_write(&json!({"drop": "c"})));
        assert!(command_is_write(&json!({"dropDatabase": 1})));
        assert!(command_is_write(
            &json!({"renameCollection": "a.b", "to": "a.c"})
        ));
        assert!(command_is_write(
            &json!({"findAndModify": "c", "query": {}, "update": {}})
        ));
        assert!(command_is_write(
            &json!({"createIndexes": "c", "indexes": []})
        ));
    }

    #[test]
    fn read_commands_allowed() {
        assert!(!command_is_write(&json!({"find": "c", "filter": {}})));
        assert!(!command_is_write(&json!({"count": "c"})));
        assert!(!command_is_write(&json!({"distinct": "c", "key": "x"})));
        assert!(!command_is_write(
            &json!({"aggregate": "c", "pipeline": [{"$match": {}}]})
        ));
        assert!(!command_is_write(&json!({"listCollections": 1})));
        assert!(!command_is_write(&json!({"dbStats": 1})));
    }

    #[test]
    fn aggregate_out_merge_is_write() {
        assert!(command_is_write(
            &json!({"aggregate": "c", "pipeline": [{"$match": {}}, {"$out": "dst"}]})
        ));
        assert!(command_is_write(
            &json!({"aggregate": "c", "pipeline": [{"$merge": {"into": "dst"}}]})
        ));
    }

    #[test]
    fn pipeline_write_stage_helper() {
        assert!(pipeline_has_write_stage(&[json!({"$out": "x"})]));
        assert!(pipeline_has_write_stage(&[json!({"$merge": {}})]));
        assert!(!pipeline_has_write_stage(&[
            json!({"$match": {}}),
            json!({"$group": {}})
        ]));
    }

    #[test]
    fn non_object_is_not_write() {
        assert!(!command_is_write(&json!("string")));
        assert!(!command_is_write(&json!(42)));
    }
}
