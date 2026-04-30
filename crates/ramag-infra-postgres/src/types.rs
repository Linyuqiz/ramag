//! PostgreSQL 类型 ↔ Domain Value 映射
//!
//! 把 sqlx::postgres::PgRow 的列值解码成 Domain `Value` enum。
//!
//! # 设计原则
//!
//! 1. 优先按列的 SQL 类型名做精确匹配
//! 2. NULL 单独处理（任何类型都可能 NULL）
//! 3. NUMERIC 用 `BigDecimal` 解码后转 `Text` 保留精度
//! 4. PG 特有类型（array / range / interval / inet / uuid 等）fallback 到 `Value::Text`，
//!    `Value` enum 不为 PG 单独扩展（与 MySQL 保持一致）
//! 5. 解码失败不 panic，fallback 到 `Value::Text` 字符串兜底

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use ramag_domain::entities::{ColumnKind, ColumnType, Value};
use sqlx::Column as _;
use sqlx::TypeInfo as _;
use sqlx::postgres::{PgColumn, PgRow};
use sqlx::types::BigDecimal;
use sqlx::types::Json as SqlxJson;
use sqlx::{Row, ValueRef};

/// 解码一行所有列为 `Vec<Value>`
pub fn decode_row(row: &PgRow) -> Vec<Value> {
    row.columns()
        .iter()
        .map(|col| decode_column(row, col))
        .collect()
}

/// 解码单列值
fn decode_column(row: &PgRow, col: &PgColumn) -> Value {
    let type_name = col.type_info().name();
    let idx = col.ordinal();

    // 1. 先判断 NULL（与具体类型无关）
    if let Ok(raw) = row.try_get_raw(idx)
        && raw.is_null()
    {
        return Value::Null;
    }

    // 2. 按 PG 类型名分发
    match type_name {
        // 布尔
        "BOOL" => row
            .try_get::<bool, _>(idx)
            .map(Value::Bool)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // 整数
        "INT2" => row
            .try_get::<i16, _>(idx)
            .map(|v| Value::Int(v as i64))
            .unwrap_or_else(|_| fallback_text(row, idx)),
        "INT4" => row
            .try_get::<i32, _>(idx)
            .map(|v| Value::Int(v as i64))
            .unwrap_or_else(|_| fallback_text(row, idx)),
        "INT8" => row
            .try_get::<i64, _>(idx)
            .map(Value::Int)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // 浮点
        "FLOAT4" => row
            .try_get::<f32, _>(idx)
            .map(|v| Value::Float(v as f64))
            .unwrap_or_else(|_| fallback_text(row, idx)),
        "FLOAT8" => row
            .try_get::<f64, _>(idx)
            .map(Value::Float)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // NUMERIC：用 BigDecimal 保留精度
        "NUMERIC" => row
            .try_get::<BigDecimal, _>(idx)
            .map(|v| Value::Text(v.to_string()))
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // 字符串
        "TEXT" | "VARCHAR" | "CHAR" | "BPCHAR" | "NAME" | "CITEXT" => row
            .try_get::<String, _>(idx)
            .map(Value::Text)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // 二进制
        "BYTEA" => row
            .try_get::<Vec<u8>, _>(idx)
            .map(Value::Bytes)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // 时间（带时区）
        "TIMESTAMPTZ" => row
            .try_get::<DateTime<Utc>, _>(idx)
            .map(Value::DateTime)
            .unwrap_or_else(|_| fallback_text(row, idx)),
        // 时间（无时区，按 UTC 处理）
        "TIMESTAMP" => row
            .try_get::<NaiveDateTime, _>(idx)
            .map(|nd| Value::DateTime(DateTime::<Utc>::from_naive_utc_and_offset(nd, Utc)))
            .unwrap_or_else(|_| fallback_text(row, idx)),
        "DATE" => row
            .try_get::<NaiveDate, _>(idx)
            .map(|d| Value::Text(d.format("%Y-%m-%d").to_string()))
            .unwrap_or_else(|_| fallback_text(row, idx)),
        "TIME" => row
            .try_get::<NaiveTime, _>(idx)
            .map(|t| Value::Text(t.format("%H:%M:%S").to_string()))
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // JSON / JSONB
        "JSON" | "JSONB" => row
            .try_get::<SqlxJson<serde_json::Value>, _>(idx)
            .map(|j| Value::Json(j.0))
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // UUID
        "UUID" => row
            .try_get::<uuid::Uuid, _>(idx)
            .map(|u| Value::Text(u.to_string()))
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // PG 特有类型（array / interval / inet / cidr / macaddr / range / time tz）→ fallback Text
        // 这些在 sqlx text protocol 下能直接 String decode（PG 把它们序列化成文本）；
        // binary protocol 下需要专属类型。fallback_text 用 raw bytes utf8 兜底
        _ => fallback_text(row, idx),
    }
}

/// 兜底：尝试当字符串读取，再失败就 NULL
fn fallback_text(row: &PgRow, idx: usize) -> Value {
    row.try_get::<String, _>(idx)
        .map(Value::Text)
        .unwrap_or(Value::Null)
}

/// PG 类型名 → Domain ColumnKind（list_columns 用）
///
/// 与 MySQL 的 map_column_type 镜像，把 PG `data_type` + 完整类型名映射到 ColumnKind
pub fn map_column_kind(data_type: &str, full_type: &str) -> ColumnType {
    let kind = match data_type.to_ascii_lowercase().as_str() {
        "boolean" => ColumnKind::Bool,
        "smallint" | "integer" | "bigint" => ColumnKind::Integer,
        "numeric" | "decimal" => ColumnKind::Decimal,
        "real" | "double precision" => ColumnKind::Float,
        "text" | "character varying" | "character" | "name" => ColumnKind::Text,
        "bytea" => ColumnKind::Blob,
        "date"
        | "timestamp without time zone"
        | "timestamp with time zone"
        | "time without time zone"
        | "time with time zone" => ColumnKind::DateTime,
        "json" | "jsonb" => ColumnKind::Json,
        _ => ColumnKind::Other,
    };
    ColumnType {
        kind,
        raw_type: full_type.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_int_types() {
        assert_eq!(
            map_column_kind("integer", "integer").kind,
            ColumnKind::Integer
        );
        assert_eq!(
            map_column_kind("bigint", "bigint").kind,
            ColumnKind::Integer
        );
        assert_eq!(
            map_column_kind("smallint", "smallint").kind,
            ColumnKind::Integer
        );
    }

    #[test]
    fn map_text_types() {
        assert_eq!(
            map_column_kind("character varying", "character varying(255)").kind,
            ColumnKind::Text
        );
        assert_eq!(map_column_kind("text", "text").kind, ColumnKind::Text);
    }

    #[test]
    fn map_decimal_keeps_precision() {
        let t = map_column_kind("numeric", "numeric(10,2)");
        assert_eq!(t.kind, ColumnKind::Decimal);
        assert_eq!(t.raw_type, "numeric(10,2)");
    }

    #[test]
    fn map_datetime_types() {
        assert_eq!(
            map_column_kind("timestamp with time zone", "timestamptz").kind,
            ColumnKind::DateTime
        );
        assert_eq!(map_column_kind("date", "date").kind, ColumnKind::DateTime);
    }

    #[test]
    fn map_jsonb() {
        assert_eq!(map_column_kind("jsonb", "jsonb").kind, ColumnKind::Json);
    }

    #[test]
    fn map_unknown_falls_to_other() {
        assert_eq!(
            map_column_kind("interval", "interval").kind,
            ColumnKind::Other
        );
        assert_eq!(map_column_kind("uuid", "uuid").kind, ColumnKind::Other);
    }
}
