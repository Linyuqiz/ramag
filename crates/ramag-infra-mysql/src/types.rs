//! MySQL 类型 ↔ Domain Value 映射
//!
//! 把 sqlx::mysql::MySqlRow 的列值解码成 Domain 的 Value enum。
//!
//! # 设计原则
//!
//! 1. 优先按列的 SQL 类型名做精确匹配（更可控）
//! 2. NULL 单独处理（任何类型的列都可能 NULL）
//! 3. DECIMAL 用 Text 保留精度，不损失到 f64
//! 4. 处理失败时不 panic，返回 Value::Text 兜底（保证总有值显示）

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use ramag_domain::entities::{ColumnKind, ColumnType, Value};
use sqlx::Column as _;
use sqlx::TypeInfo as _;
use sqlx::mysql::{MySqlColumn, MySqlRow};
use sqlx::{Row, ValueRef};

/// 解码一行所有列为 `Vec<Value>`
pub fn decode_row(row: &MySqlRow) -> Vec<Value> {
    row.columns()
        .iter()
        .map(|col| decode_column(row, col))
        .collect()
}

/// 解码单列值
fn decode_column(row: &MySqlRow, col: &MySqlColumn) -> Value {
    let type_name = col.type_info().name();
    let idx = col.ordinal();

    // 1. 先判断是否为 NULL（无关具体类型）
    if let Ok(raw) = row.try_get_raw(idx)
        && raw.is_null()
    {
        return Value::Null;
    }

    // 2. 按 MySQL 类型名分发
    match type_name {
        // 布尔
        "BOOLEAN" => row
            .try_get::<bool, _>(idx)
            .map(Value::Bool)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // 整数家族
        "TINYINT" => decode_int::<i8>(row, idx),
        "TINYINT UNSIGNED" => decode_int::<u8>(row, idx),
        "SMALLINT" => decode_int::<i16>(row, idx),
        "SMALLINT UNSIGNED" => decode_int::<u16>(row, idx),
        "MEDIUMINT" => decode_int::<i32>(row, idx),
        "MEDIUMINT UNSIGNED" => decode_int::<u32>(row, idx),
        "INT" | "INTEGER" => decode_int::<i32>(row, idx),
        "INT UNSIGNED" | "INTEGER UNSIGNED" => decode_int::<u32>(row, idx),
        "BIGINT" => decode_int::<i64>(row, idx),
        "BIGINT UNSIGNED" => row
            .try_get::<u64, _>(idx)
            .map(|v| {
                // u64 可能溢出 i64，超大值用 Text 保留
                if v > i64::MAX as u64 {
                    Value::Text(v.to_string())
                } else {
                    Value::Int(v as i64)
                }
            })
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // 浮点
        "FLOAT" => row
            .try_get::<f32, _>(idx)
            .map(|v| Value::Float(v as f64))
            .unwrap_or_else(|_| fallback_text(row, idx)),
        "DOUBLE" => row
            .try_get::<f64, _>(idx)
            .map(Value::Float)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // DECIMAL：用字符串保留精度
        "DECIMAL" | "NUMERIC" => row
            .try_get::<String, _>(idx)
            .map(Value::Text)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // 字符串
        "CHAR" | "VARCHAR" | "TEXT" | "TINYTEXT" | "MEDIUMTEXT" | "LONGTEXT" => row
            .try_get::<String, _>(idx)
            .map(Value::Text)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // 二进制
        "BINARY" | "VARBINARY" | "BLOB" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" | "BIT" => row
            .try_get::<Vec<u8>, _>(idx)
            .map(Value::Bytes)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // 时间
        "DATETIME" => row
            .try_get::<NaiveDateTime, _>(idx)
            .map(|nd| Value::DateTime(DateTime::<Utc>::from_naive_utc_and_offset(nd, Utc)))
            .unwrap_or_else(|_| fallback_text(row, idx)),
        "TIMESTAMP" => row
            .try_get::<DateTime<Utc>, _>(idx)
            .map(Value::DateTime)
            .unwrap_or_else(|_| fallback_text(row, idx)),
        "DATE" => row
            .try_get::<NaiveDate, _>(idx)
            .map(|d| Value::Text(d.format("%Y-%m-%d").to_string()))
            .unwrap_or_else(|_| fallback_text(row, idx)),
        "TIME" => row
            .try_get::<NaiveTime, _>(idx)
            .map(|t| Value::Text(t.format("%H:%M:%S").to_string()))
            .unwrap_or_else(|_| fallback_text(row, idx)),
        "YEAR" => decode_int::<u16>(row, idx),

        // JSON
        "JSON" => row
            .try_get::<serde_json::Value, _>(idx)
            .map(Value::Json)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // ENUM/SET：MySQL 内部是字符串
        "ENUM" | "SET" => row
            .try_get::<String, _>(idx)
            .map(Value::Text)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // 未知类型：尝试当字符串处理
        _ => fallback_text(row, idx),
    }
}

fn decode_int<T>(row: &MySqlRow, idx: usize) -> Value
where
    T: for<'r> sqlx::Decode<'r, sqlx::MySql> + sqlx::Type<sqlx::MySql> + Into<i64>,
{
    row.try_get::<T, _>(idx)
        .map(|v| Value::Int(v.into()))
        .unwrap_or_else(|_| fallback_text(row, idx))
}

/// 兜底：尝试当字符串读取，再失败就 Null
fn fallback_text(row: &MySqlRow, idx: usize) -> Value {
    row.try_get::<String, _>(idx)
        .map(Value::Text)
        .unwrap_or(Value::Null)
}

/// MySQL `data_type` + `column_type` → Domain ColumnType
///
/// 用于 list_columns 时把 INFORMATION_SCHEMA.COLUMNS 的列类型映射到 ColumnKind
pub fn map_column_type(data_type: &str, column_type: &str) -> ColumnType {
    let kind = match data_type.to_ascii_uppercase().as_str() {
        "TINYINT" if column_type.eq_ignore_ascii_case("tinyint(1)") => ColumnKind::Bool,
        "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "INTEGER" | "BIGINT" | "YEAR" => {
            ColumnKind::Integer
        }
        "DECIMAL" | "NUMERIC" => ColumnKind::Decimal,
        "FLOAT" | "DOUBLE" | "REAL" => ColumnKind::Float,
        "CHAR" | "VARCHAR" | "TEXT" | "TINYTEXT" | "MEDIUMTEXT" | "LONGTEXT" | "ENUM" | "SET" => {
            ColumnKind::Text
        }
        "BINARY" | "VARBINARY" | "BLOB" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" | "BIT" => {
            ColumnKind::Blob
        }
        "DATE" | "DATETIME" | "TIMESTAMP" | "TIME" => ColumnKind::DateTime,
        "JSON" => ColumnKind::Json,
        _ => ColumnKind::Other,
    };

    ColumnType {
        kind,
        raw_type: column_type.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_int_types() {
        assert_eq!(map_column_type("INT", "int(11)").kind, ColumnKind::Integer);
        assert_eq!(
            map_column_type("BIGINT", "bigint(20)").kind,
            ColumnKind::Integer
        );
        assert_eq!(map_column_type("YEAR", "year").kind, ColumnKind::Integer);
    }

    #[test]
    fn map_tinyint_one_is_bool() {
        // TINYINT(1) 习惯上当布尔
        assert_eq!(
            map_column_type("TINYINT", "tinyint(1)").kind,
            ColumnKind::Bool
        );
        // TINYINT(4) 是整数
        assert_eq!(
            map_column_type("TINYINT", "tinyint(4)").kind,
            ColumnKind::Integer
        );
    }

    #[test]
    fn map_text_types() {
        assert_eq!(
            map_column_type("VARCHAR", "varchar(255)").kind,
            ColumnKind::Text
        );
        assert_eq!(
            map_column_type("LONGTEXT", "longtext").kind,
            ColumnKind::Text
        );
        assert_eq!(
            map_column_type("ENUM", "enum('a','b')").kind,
            ColumnKind::Text
        );
    }

    #[test]
    fn map_blob_types() {
        assert_eq!(map_column_type("BLOB", "blob").kind, ColumnKind::Blob);
        assert_eq!(map_column_type("BIT", "bit(8)").kind, ColumnKind::Blob);
    }

    #[test]
    fn map_datetime_types() {
        assert_eq!(
            map_column_type("DATETIME", "datetime").kind,
            ColumnKind::DateTime
        );
        assert_eq!(
            map_column_type("TIMESTAMP", "timestamp").kind,
            ColumnKind::DateTime
        );
        assert_eq!(map_column_type("DATE", "date").kind, ColumnKind::DateTime);
    }

    #[test]
    fn map_json() {
        assert_eq!(map_column_type("JSON", "json").kind, ColumnKind::Json);
    }

    #[test]
    fn map_decimal_keeps_precision() {
        let t = map_column_type("DECIMAL", "decimal(10,2)");
        assert_eq!(t.kind, ColumnKind::Decimal);
        assert_eq!(t.raw_type, "decimal(10,2)");
    }

    #[test]
    fn map_unknown() {
        assert_eq!(
            map_column_type("GEOMETRY", "geometry").kind,
            ColumnKind::Other
        );
    }
}
