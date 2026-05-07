//! MySQL 行解码：MySqlRow → Domain Value。按 SQL 类型名精确分发；DECIMAL 用 Text 保精度；失败 Text 兜底

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use ramag_domain::entities::{ColumnKind, ColumnType, Value};
use sqlx::Column as _;
use sqlx::TypeInfo as _;
use sqlx::mysql::{MySqlColumn, MySqlRow};
use sqlx::{Row, ValueRef};

pub fn decode_row(row: &MySqlRow) -> Vec<Value> {
    row.columns()
        .iter()
        .map(|col| decode_column(row, col))
        .collect()
}

fn decode_column(row: &MySqlRow, col: &MySqlColumn) -> Value {
    let type_name = col.type_info().name();
    let idx = col.ordinal();

    if let Ok(raw) = row.try_get_raw(idx)
        && raw.is_null()
    {
        return Value::Null;
    }

    match type_name {
        "BOOLEAN" => row
            .try_get::<bool, _>(idx)
            .map(Value::Bool)
            .unwrap_or_else(|_| fallback_text(row, idx)),

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
                // u64 超 i64::MAX 时用 Text 保值
                if v > i64::MAX as u64 {
                    Value::Text(v.to_string())
                } else {
                    Value::Int(v as i64)
                }
            })
            .unwrap_or_else(|_| fallback_text(row, idx)),

        "FLOAT" => row
            .try_get::<f32, _>(idx)
            .map(|v| Value::Float(v as f64))
            .unwrap_or_else(|_| fallback_text(row, idx)),
        "DOUBLE" => row
            .try_get::<f64, _>(idx)
            .map(Value::Float)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // DECIMAL：字符串保精度
        "DECIMAL" | "NUMERIC" => row
            .try_get::<String, _>(idx)
            .map(Value::Text)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        "CHAR" | "VARCHAR" | "TEXT" | "TINYTEXT" | "MEDIUMTEXT" | "LONGTEXT" => row
            .try_get::<String, _>(idx)
            .map(Value::Text)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        "BINARY" | "VARBINARY" | "BLOB" | "TINYBLOB" | "MEDIUMBLOB" | "LONGBLOB" | "BIT" => row
            .try_get::<Vec<u8>, _>(idx)
            .map(Value::Bytes)
            .unwrap_or_else(|_| fallback_text(row, idx)),

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

        "JSON" => row
            .try_get::<serde_json::Value, _>(idx)
            .map(Value::Json)
            .unwrap_or_else(|_| fallback_text(row, idx)),

        // ENUM/SET 内部存字符串
        "ENUM" | "SET" => row
            .try_get::<String, _>(idx)
            .map(Value::Text)
            .unwrap_or_else(|_| fallback_text(row, idx)),

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

/// 当字符串读，再失败 Null
fn fallback_text(row: &MySqlRow, idx: usize) -> Value {
    row.try_get::<String, _>(idx)
        .map(Value::Text)
        .unwrap_or(Value::Null)
}

/// 把 INFORMATION_SCHEMA.COLUMNS 的 (data_type, column_type) 映射到 ColumnKind
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
        assert_eq!(
            map_column_type("TINYINT", "tinyint(1)").kind,
            ColumnKind::Bool
        );
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
