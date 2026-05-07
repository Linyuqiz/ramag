//! 元数据查询：基于 INFORMATION_SCHEMA，避免 SHOW 语法的版本差异。
//! 字符串列统一 `CONVERT(... USING utf8mb4)`，避开 sqlx 把某些环境的回包识为 VARBINARY 导致解码失败

use ramag_domain::entities::{Column, ForeignKey, Index, Schema, Table};
use ramag_domain::error::Result;
use sqlx::MySqlPool;
use tracing::debug;

use crate::errors::map_mysql_error;
use crate::types::map_column_type;

/// 含系统库（mysql / information_schema / performance_schema / sys）；过滤交给 UI
pub async fn list_schemas(pool: &MySqlPool) -> Result<Vec<Schema>> {
    debug!("list_schemas");

    let rows: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        r#"
        SELECT
            CONVERT(SCHEMA_NAME USING utf8mb4),
            CONVERT(DEFAULT_CHARACTER_SET_NAME USING utf8mb4),
            CONVERT(DEFAULT_COLLATION_NAME USING utf8mb4)
        FROM information_schema.SCHEMATA
        ORDER BY SCHEMA_NAME
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| map_mysql_error(&e))?;

    Ok(rows
        .into_iter()
        .map(|(name, charset, collation)| Schema {
            name,
            charset,
            collation,
        })
        .collect())
}

/// 列出 BASE TABLE / VIEW / SYSTEM VIEW。后两者在 UI 都归为视图分组
pub async fn list_tables(pool: &MySqlPool, schema: &str) -> Result<Vec<Table>> {
    debug!(?schema, "list_tables");

    let rows: Vec<(String, String, Option<String>, Option<u64>)> = sqlx::query_as(
        r#"
        SELECT
            CONVERT(TABLE_NAME USING utf8mb4),
            CONVERT(TABLE_TYPE USING utf8mb4),
            CONVERT(TABLE_COMMENT USING utf8mb4),
            TABLE_ROWS
        FROM information_schema.TABLES
        WHERE TABLE_SCHEMA = ? AND TABLE_TYPE IN ('BASE TABLE', 'VIEW', 'SYSTEM VIEW')
        ORDER BY TABLE_TYPE, TABLE_NAME
        "#,
    )
    .bind(schema)
    .fetch_all(pool)
    .await
    .map_err(|e| map_mysql_error(&e))?;

    Ok(rows
        .into_iter()
        .map(|(name, table_type, comment, row_estimate)| {
            let is_view = !table_type.eq_ignore_ascii_case("BASE TABLE");
            // VIEW 多数 NULL，服务端没给数就当 0；精确行数让用户走 SELECT COUNT(*)
            let row_estimate = Some(row_estimate.unwrap_or(0));
            Table {
                name,
                schema: schema.to_string(),
                comment: comment.filter(|c| !c.is_empty()),
                row_estimate,
                is_view,
            }
        })
        .collect())
}

/// COLUMNS 一行：name / data_type / column_type / is_nullable / column_default / column_comment / column_key
type ColumnRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
);

/// 列出指定表的所有列
pub async fn list_columns(pool: &MySqlPool, schema: &str, table: &str) -> Result<Vec<Column>> {
    debug!(?schema, ?table, "list_columns");

    let rows: Vec<ColumnRow> = sqlx::query_as(
        r#"
            SELECT
                CONVERT(COLUMN_NAME USING utf8mb4),
                CONVERT(DATA_TYPE USING utf8mb4),
                CONVERT(COLUMN_TYPE USING utf8mb4),
                CONVERT(IS_NULLABLE USING utf8mb4),
                CONVERT(COLUMN_DEFAULT USING utf8mb4),
                CONVERT(COLUMN_COMMENT USING utf8mb4),
                CONVERT(COLUMN_KEY USING utf8mb4)
            FROM information_schema.COLUMNS
            WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
            ORDER BY ORDINAL_POSITION
            "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| map_mysql_error(&e))?;

    Ok(rows
        .into_iter()
        .map(
            |(name, data_type, column_type, is_nullable, default_value, comment, column_key)| {
                Column {
                    name,
                    data_type: map_column_type(&data_type, &column_type),
                    nullable: is_nullable.eq_ignore_ascii_case("YES"),
                    default_value,
                    is_primary_key: column_key == "PRI",
                    comment: comment.filter(|c| !c.is_empty()),
                }
            },
        )
        .collect())
}

/// 含主键 / 唯一 / 普通索引。基于 STATISTICS 一行一列，按 INDEX_NAME 聚合
pub async fn list_indexes(pool: &MySqlPool, schema: &str, table: &str) -> Result<Vec<Index>> {
    debug!(?schema, ?table, "list_indexes");

    let rows: Vec<(String, i64, i64, String)> = sqlx::query_as(
        r#"
        SELECT
            CONVERT(INDEX_NAME USING utf8mb4),
            CAST(NON_UNIQUE AS SIGNED),
            CAST(SEQ_IN_INDEX AS SIGNED),
            CONVERT(COLUMN_NAME USING utf8mb4)
        FROM information_schema.STATISTICS
        WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
        ORDER BY INDEX_NAME, SEQ_IN_INDEX
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| map_mysql_error(&e))?;

    let mut grouped: std::collections::BTreeMap<String, Index> = std::collections::BTreeMap::new();
    for (idx_name, non_unique, _seq, col_name) in rows {
        let primary = idx_name == "PRIMARY";
        let entry = grouped.entry(idx_name.clone()).or_insert_with(|| Index {
            name: idx_name,
            unique: non_unique == 0,
            primary,
            columns: Vec::new(),
        });
        entry.columns.push(col_name);
    }

    // 主键置顶，其余按名
    let mut indexes: Vec<Index> = grouped.into_values().collect();
    indexes.sort_by(|a, b| match (a.primary, b.primary) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });
    Ok(indexes)
}

/// 基于 KEY_COLUMN_USAGE
pub async fn list_foreign_keys(
    pool: &MySqlPool,
    schema: &str,
    table: &str,
) -> Result<Vec<ForeignKey>> {
    debug!(?schema, ?table, "list_foreign_keys");

    let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        r#"
        SELECT
            CONVERT(CONSTRAINT_NAME USING utf8mb4),
            CONVERT(COLUMN_NAME USING utf8mb4),
            CONVERT(REFERENCED_TABLE_SCHEMA USING utf8mb4),
            CONVERT(REFERENCED_TABLE_NAME USING utf8mb4),
            CONVERT(REFERENCED_COLUMN_NAME USING utf8mb4)
        FROM information_schema.KEY_COLUMN_USAGE
        WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?
          AND REFERENCED_TABLE_NAME IS NOT NULL
        ORDER BY CONSTRAINT_NAME, ORDINAL_POSITION
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| map_mysql_error(&e))?;

    let mut grouped: std::collections::BTreeMap<String, ForeignKey> =
        std::collections::BTreeMap::new();
    for (name, col, ref_schema, ref_table, ref_col) in rows {
        let entry = grouped.entry(name.clone()).or_insert_with(|| ForeignKey {
            name,
            columns: Vec::new(),
            ref_schema,
            ref_table,
            ref_columns: Vec::new(),
        });
        entry.columns.push(col);
        entry.ref_columns.push(ref_col);
    }
    Ok(grouped.into_values().collect())
}

/// SELECT 1
pub async fn ping(pool: &MySqlPool) -> Result<()> {
    let _: (i64,) = sqlx::query_as("SELECT 1")
        .fetch_one(pool)
        .await
        .map_err(|e| map_mysql_error(&e))?;
    Ok(())
}

/// `SELECT VERSION()`，形如 "8.0.32"
pub async fn server_version(pool: &MySqlPool) -> Result<String> {
    let (v,): (String,) = sqlx::query_as("SELECT VERSION()")
        .fetch_one(pool)
        .await
        .map_err(|e| map_mysql_error(&e))?;
    Ok(v)
}
