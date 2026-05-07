//! PG 行解码：PgRow → Domain Value。NUMERIC 用 BigDecimal 转 Text 保精度；array/interval/inet/uuid 等 fallback Text

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use ramag_domain::entities::{ColumnKind, ColumnType, Value};
use sqlx::Column as _;
use sqlx::TypeInfo as _;
use sqlx::postgres::{PgColumn, PgRow};
use sqlx::types::BigDecimal;
use sqlx::types::Json as SqlxJson;
use sqlx::{Row, ValueRef};

pub fn decode_row(row: &PgRow) -> Vec<Value> {
    row.columns()
        .iter()
        .map(|col| decode_column(row, col))
        .collect()
}

fn decode_column(row: &PgRow, col: &PgColumn) -> Value {
    let type_name = col.type_info().name();
    let idx = col.ordinal();

    if let Ok(raw) = row.try_get_raw(idx)
        && raw.is_null()
    {
        return Value::Null;
    }

    match type_name {
        "BOOL" => row
            .try_get::<bool, _>(idx)
            .map(Value::Bool)
            .unwrap_or_else(|_| fallback_text(row, idx)),

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

        "FLOAT4" => row
            .try_get::<f32, _>(idx)
            .map(|v| Value::Float(v as f64))
            .unwrap_or_else(|_| fallback_text(row, idx)),
        "FLOAT8" => row
            .try_get::<f64, _>(idx)
            .map(Value::Float)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // BigDecimal 保精度
        "NUMERIC" => row
            .try_get::<BigDecimal, _>(idx)
            .map(|v| Value::Text(v.to_string()))
            .unwrap_or_else(|_| fallback_text(row, idx)),

        "TEXT" | "VARCHAR" | "CHAR" | "BPCHAR" | "NAME" | "CITEXT" => row
            .try_get::<String, _>(idx)
            .map(Value::Text)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        "BYTEA" => row
            .try_get::<Vec<u8>, _>(idx)
            .map(Value::Bytes)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // 带时区
        "TIMESTAMPTZ" => row
            .try_get::<DateTime<Utc>, _>(idx)
            .map(Value::DateTime)
            .unwrap_or_else(|_| fallback_text(row, idx)),
        // 无时区按 UTC
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

        "JSON" | "JSONB" => row
            .try_get::<SqlxJson<serde_json::Value>, _>(idx)
            .map(|j| Value::Json(j.0))
            .unwrap_or_else(|_| fallback_text(row, idx)),

        "UUID" => row
            .try_get::<uuid::Uuid, _>(idx)
            .map(|u| Value::Text(u.to_string()))
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // PG 特有类型（array / range / interval / inet / cidr / macaddr / time tz）走 String 文本兜底
        _ => fallback_text(row, idx),
    }
}

/// 当字符串读，再失败 NULL
fn fallback_text(row: &PgRow, idx: usize) -> Value {
    row.try_get::<String, _>(idx)
        .map(Value::Text)
        .unwrap_or(Value::Null)
}

/// 把 information_schema 的 (data_type, full_type) 映射到 ColumnKind
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
