//! MongoDB 文档数据库实体。文档以 serde_json::Value 表达，
//! infra 层负责 BSON ↔ Extended JSON 双向映射（ObjectId → `{"$oid":...}` 等）

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 数据库
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MongoDatabase {
    pub name: String,
    /// listDatabases 给出的字节数；admin 库或受限场景可能为 None
    pub size_on_disk: Option<u64>,
    pub empty: bool,
}

/// 集合（含 view 兼容）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MongoCollection {
    pub name: String,
    pub database: String,
    /// 视图，无法写入
    pub is_view: bool,
}

/// 索引。`keys` 保留 spec 顺序（复合索引语义敏感）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MongoIndex {
    pub name: String,
    /// (字段名, 方向)；方向 1=升序 / -1=降序 / 文本索引等扩展为 0
    pub keys: Vec<(String, i32)>,
    pub unique: bool,
    /// `_id` 索引视为主键
    pub primary: bool,
    pub sparse: bool,
}

/// MongoDB 文档。Extended JSON 风格，
/// `ObjectId → {"$oid": "..."}`、`Decimal128 → {"$numberDecimal": "..."}`、
/// `DateTime → {"$date": "ISO8601"}`、`Binary → {"$binary": {"base64": "...", "subType": "..."}}`
pub type MongoDocument = Value;

/// `find` 查询规格
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MongoQuerySpec {
    /// filter（必须为 JSON 对象，空对象 = 匹配全部）
    pub filter: Value,
    /// 投影，None = 全部字段
    pub projection: Option<Value>,
    /// 排序 spec，例 `{"createdAt": -1}`
    pub sort: Option<Value>,
    /// 跳过文档数（分页）
    pub skip: Option<u64>,
    /// 返回上限。None 走 UI 默认值
    pub limit: Option<i64>,
}

/// 查询结果。无论 read / write 都用同一结构上报
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MongoQueryResult {
    /// read 类返回的文档；write 类为空
    pub documents: Vec<MongoDocument>,
    /// write 类返回的影响数（matched / modified / deleted / inserted）
    pub affected: u64,
    pub elapsed_ms: u64,
    /// UI 状态栏 / 历史摘要，如 "12 docs, 18ms"
    pub summary: String,
}

/// 集合统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MongoCollectionStats {
    pub count: u64,
    pub size_bytes: u64,
    pub avg_obj_size: u64,
    pub storage_size: u64,
    pub index_count: u32,
}

impl MongoQueryResult {
    /// read 类构造（拼摘要）
    pub fn read(documents: Vec<MongoDocument>, elapsed_ms: u64) -> Self {
        let n = documents.len();
        Self {
            documents,
            affected: 0,
            elapsed_ms,
            summary: format!("{n} docs, {elapsed_ms}ms"),
        }
    }

    /// write 类构造
    pub fn write(affected: u64, elapsed_ms: u64, op: &str) -> Self {
        Self {
            documents: Vec::new(),
            affected,
            elapsed_ms,
            summary: format!("{op} affected={affected}, {elapsed_ms}ms"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn read_summary_includes_count() {
        let r = MongoQueryResult::read(vec![json!({"a": 1}), json!({"a": 2})], 5);
        assert_eq!(r.documents.len(), 2);
        assert_eq!(r.affected, 0);
        assert!(r.summary.contains("2 docs"));
    }

    #[test]
    fn write_summary_includes_op() {
        let r = MongoQueryResult::write(3, 12, "updateOne");
        assert!(r.summary.contains("updateOne"));
        assert!(r.summary.contains("affected=3"));
    }
}
