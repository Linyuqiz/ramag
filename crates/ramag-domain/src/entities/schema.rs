//! 元数据实体（库 / 表 / 列）

use serde::{Deserialize, Serialize};

/// 一个数据库实例下的"库 / schema"概念
///
/// MySQL 里 schema == database，一个实例下可以有多个；
/// PostgreSQL 里一个 database 下可以有多个 schema；
/// 本结构对两者都适配
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub name: String,
    pub charset: Option<String>,
    pub collation: Option<String>,
}

/// 表 / 视图
///
/// 用 `is_view` 区分基础表（BASE TABLE）和视图（VIEW）。两者元数据结构基本相同，
/// 仅元数据来源 SQL 不同（视图无 row_estimate / comment 通常为空），UI 用 `is_view` 切换图标和分组。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Table {
    pub name: String,
    pub schema: String,
    /// 表注释
    pub comment: Option<String>,
    /// 估算的行数（INFORMATION_SCHEMA 提供，不一定准确）；视图为 None
    pub row_estimate: Option<u64>,
    /// 是否视图（对应 INFORMATION_SCHEMA.TABLES.TABLE_TYPE = 'VIEW'）
    /// 用 serde default 兼容老数据：旧持久化记录里没这个字段时反序列化为 false
    #[serde(default)]
    pub is_view: bool,
}

/// 列定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub data_type: ColumnType,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub is_primary_key: bool,
    pub comment: Option<String>,
}

/// 列类型（跨数据库统一抽象）
///
/// 具体的 INT(11)、VARCHAR(255) 等细节通过 raw_type 字符串保留
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnType {
    /// 抽象后的类型分类
    pub kind: ColumnKind,
    /// 原始类型字符串（如 "VARCHAR(255)"、"DECIMAL(10,2)"）
    pub raw_type: String,
}

/// 列类型分类（用于 UI 选择渲染方式 / 编辑器）
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
    /// 未识别 / 数据库特有类型
    Other,
}

/// 索引定义（含主键、唯一索引、普通索引）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    pub name: String,
    /// 是否唯一（主键也算唯一）
    pub unique: bool,
    /// 是否主键
    pub primary: bool,
    /// 索引涉及的列名（按顺序）
    pub columns: Vec<String>,
}

/// 外键定义（references 父表）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKey {
    pub name: String,
    /// 本表涉及的列
    pub columns: Vec<String>,
    /// 引用的目标库
    pub ref_schema: String,
    /// 引用的目标表
    pub ref_table: String,
    /// 引用的目标列（与 columns 等长）
    pub ref_columns: Vec<String>,
}
