//! 元数据实体：库 / 表 / 列

use serde::{Deserialize, Serialize};

/// MySQL 的 schema==database，PG 的 database 下可有多 schema，本结构两者通用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub name: String,
    pub charset: Option<String>,
    pub collation: Option<String>,
}

/// 表 / 视图，由 `is_view` 区分
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Table {
    pub name: String,
    pub schema: String,
    pub comment: Option<String>,
    /// 兼容老持久化记录，缺字段时 false
    #[serde(default)]
    pub is_view: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub data_type: ColumnType,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub is_primary_key: bool,
    pub comment: Option<String>,
}

/// 列类型。`raw_type` 保留 `VARCHAR(255)` / `DECIMAL(10,2)` 等原始细节
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnType {
    pub kind: ColumnKind,
    pub raw_type: String,
}

/// 列类型分类（驱动 UI 编辑器选择）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColumnKind {
    Integer,
    Decimal,
    Float,
    Text,
    Blob,
    Bool,
    DateTime,
    Json,
    /// 未识别 / 数据库特有
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    pub name: String,
    /// 主键也算唯一
    pub unique: bool,
    pub primary: bool,
    /// 索引列，按顺序
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKey {
    pub name: String,
    /// 本表涉及的列
    pub columns: Vec<String>,
    pub ref_schema: String,
    pub ref_table: String,
    /// 与 columns 等长
    pub ref_columns: Vec<String>,
}
