//! 元数据查询：列出 schemas / tables / columns
//!
//! 全部基于 INFORMATION_SCHEMA，避免依赖具体 MySQL 版本的 SHOW 语法差异。

use ramag_domain::entities::{Column, ForeignKey, Index, Schema, Table};
use ramag_domain::error::Result;
use sqlx::MySqlPool;
use tracing::debug;

use crate::errors::map_sqlx_error;
use crate::types::map_column_type;

// 注意：MySQL 的 INFORMATION_SCHEMA 列定义为 utf8 而 sqlx 把某些环境下的回包识别成
// VARBINARY，导致 String 解码失败。统一用 CONVERT(... USING utf8mb4) 强制成字符串类型，
// 避开类型不匹配。

/// 列出所有 schemas（库），**包含系统库**
///
/// 系统库（mysql / information_schema / performance_schema / sys）由 UI 层自行过滤，
/// 这里全部返回，让上层灵活控制显示。
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
    .map_err(map_sqlx_error)?;

    Ok(rows
        .into_iter()
        .map(|(name, charset, collation)| Schema {
            name,
            charset,
            collation,
        })
        .collect())
}

/// 列出指定 schema 下的所有 BASE TABLE / VIEW / SYSTEM VIEW
///
/// TABLE_TYPE 含义：
/// - `BASE TABLE`：普通用户表
/// - `VIEW`：用户定义的视图
/// - `SYSTEM VIEW`：information_schema 等系统库内的动态视图（服务端实时生成）
///
/// 后两者在 UI 层都用 Frame 图标 + "视图" 分组（`is_view = true`），
/// 让 information_schema / sys 也能正常展开浏览
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
    .map_err(map_sqlx_error)?;

    Ok(rows
        .into_iter()
        .map(|(name, table_type, comment, row_estimate)| {
            // 非 BASE TABLE 一律归为视图：覆盖 VIEW + SYSTEM VIEW 两种
            // 它们在 UI 上的渲染（图标 / DDL 行为 / 行为差异）一致
            let is_view = !table_type.eq_ignore_ascii_case("BASE TABLE");
            // 所有表（含 VIEW / SYSTEM VIEW）行为统一：都显示行数估算
            // NULL（VIEW 多数）↦ 0：服务端没给数就当 0，UI 显示 (~0)
            // 这与 SYSTEM VIEW 真实给的 0 视觉一致；不精确就不精确（用户要求）
            // 精确行数请 SELECT COUNT(*)
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

/// 列出指定表的所有列
pub async fn list_columns(pool: &MySqlPool, schema: &str, table: &str) -> Result<Vec<Column>> {
    debug!(?schema, ?table, "list_columns");

    let rows: Vec<(
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        String,
    )> = sqlx::query_as(
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
    .map_err(map_sqlx_error)?;

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

/// 列出指定表的所有索引（含主键、唯一、普通）
/// INFORMATION_SCHEMA.STATISTICS 一行一列，按 INDEX_NAME 聚合
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
    .map_err(map_sqlx_error)?;

    // 按 INDEX_NAME 聚合 columns
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

    // 主键排第一，其它按名字
    let mut indexes: Vec<Index> = grouped.into_values().collect();
    indexes.sort_by(|a, b| match (a.primary, b.primary) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });
    Ok(indexes)
}

/// 列出指定表的所有外键
/// 用 KEY_COLUMN_USAGE join REFERENTIAL_CONSTRAINTS
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
    .map_err(map_sqlx_error)?;

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

/// SELECT 1 测试连接是否可用
pub async fn ping(pool: &MySqlPool) -> Result<()> {
    let _: (i64,) = sqlx::query_as("SELECT 1")
        .fetch_one(pool)
        .await
        .map_err(map_sqlx_error)?;
    Ok(())
}

/// SELECT VERSION() 取服务端版本字符串
///
/// 形如 "8.0.32" 或 "5.7.40-log"。UI 在连接列表里展示，便于区分实例
pub async fn server_version(pool: &MySqlPool) -> Result<String> {
    let (v,): (String,) = sqlx::query_as("SELECT VERSION()")
        .fetch_one(pool)
        .await
        .map_err(map_sqlx_error)?;
    Ok(v)
}
