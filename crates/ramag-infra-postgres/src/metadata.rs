//! PG 元数据：优先 information_schema，少量索引/列注释走 pg_catalog

use ramag_domain::entities::{Column, ForeignKey, Index, Schema, Table};
use ramag_domain::error::Result;
use sqlx::PgPool;
use tracing::debug;

use crate::errors::map_postgres_error;
use crate::types::map_column_kind;

/// 含系统 schema（pg_catalog / information_schema / pg_toast / pg_temp_*）；过滤交给 UI
pub async fn list_schemas(pool: &PgPool) -> Result<Vec<Schema>> {
    debug!("list_schemas (postgres)");

    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        r#"
        SELECT schema_name, default_character_set_name
        FROM information_schema.schemata
        ORDER BY schema_name
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| map_postgres_error(&e))?;

    Ok(rows
        .into_iter()
        .map(|(name, charset)| Schema {
            name,
            charset,
            // PG 的 collation 是列 / 表级，schema 无此概念
            collation: None,
        })
        .collect())
}

/// 列出 BASE TABLE / VIEW / MATERIALIZED VIEW。matview 不在 information_schema.tables，需 union pg_matviews
pub async fn list_tables(pool: &PgPool, schema: &str) -> Result<Vec<Table>> {
    debug!(?schema, "list_tables (postgres)");

    let rows: Vec<(String, String, Option<String>, Option<i64>)> = sqlx::query_as(
        r#"
        SELECT
            t.table_name::text,
            t.table_type::text,
            obj_description(c.oid, 'pg_class') AS table_comment,
            c.reltuples::bigint AS row_estimate
        FROM information_schema.tables t
        LEFT JOIN pg_namespace n ON n.nspname = t.table_schema
        LEFT JOIN pg_class c ON c.relnamespace = n.oid AND c.relname = t.table_name
        WHERE t.table_schema = $1
          AND t.table_type IN ('BASE TABLE', 'VIEW')
        UNION ALL
        SELECT
            mv.matviewname::text AS table_name,
            'MATERIALIZED VIEW'::text AS table_type,
            obj_description(c.oid, 'pg_class') AS table_comment,
            c.reltuples::bigint AS row_estimate
        FROM pg_matviews mv
        LEFT JOIN pg_namespace n ON n.nspname = mv.schemaname
        LEFT JOIN pg_class c ON c.relnamespace = n.oid AND c.relname = mv.matviewname
        WHERE mv.schemaname = $1
        ORDER BY 2, 1
        "#,
    )
    .bind(schema)
    .fetch_all(pool)
    .await
    .map_err(|e| map_postgres_error(&e))?;

    Ok(rows
        .into_iter()
        .map(|(name, table_type, comment, row_estimate)| {
            let is_view = !table_type.eq_ignore_ascii_case("BASE TABLE");
            // reltuples 估算值，负数表示未分析，归零
            let row_estimate = Some(row_estimate.map(|v| v.max(0) as u64).unwrap_or(0));
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

/// COLUMNS 一行：column_name / data_type / udt_name / default / comment / char_max_len / nullable
type PgColumnRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<i32>,
    bool,
);

/// 列注释走 pg_catalog.col_description，其他走 information_schema.columns
pub async fn list_columns(pool: &PgPool, schema: &str, table: &str) -> Result<Vec<Column>> {
    debug!(?schema, ?table, "list_columns (postgres)");

    let rows: Vec<PgColumnRow> = sqlx::query_as(
        r#"
        SELECT
            c.column_name::text,
            c.data_type::text,
            c.udt_name::text,
            c.column_default,
            col_description(pgc.oid, c.ordinal_position::int) AS column_comment,
            c.character_maximum_length::int,
            (c.is_nullable = 'YES') AS nullable
        FROM information_schema.columns c
        LEFT JOIN pg_namespace n ON n.nspname = c.table_schema
        LEFT JOIN pg_class pgc ON pgc.relnamespace = n.oid AND pgc.relname = c.table_name
        WHERE c.table_schema = $1 AND c.table_name = $2
        ORDER BY c.ordinal_position
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| map_postgres_error(&e))?;

    // 主键列另查一次 key_column_usage + table_constraints
    let pk_cols: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT kcu.column_name::text
        FROM information_schema.table_constraints tc
        JOIN information_schema.key_column_usage kcu
          ON tc.constraint_name = kcu.constraint_name
         AND tc.table_schema = kcu.table_schema
        WHERE tc.constraint_type = 'PRIMARY KEY'
          AND tc.table_schema = $1 AND tc.table_name = $2
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| map_postgres_error(&e))?;
    let pk_names: std::collections::HashSet<String> = pk_cols.into_iter().map(|(n,)| n).collect();

    Ok(rows
        .into_iter()
        .map(
            |(name, data_type, udt_name, default_value, comment, char_max_len, nullable)| {
                // PG information_schema 没现成完整类型列，手动拼 varchar(255) / numeric(10,2)
                let full_type = compose_full_type(&data_type, &udt_name, char_max_len);
                Column {
                    name: name.clone(),
                    data_type: map_column_kind(&data_type, &full_type),
                    nullable,
                    default_value,
                    is_primary_key: pk_names.contains(&name),
                    comment: comment.filter(|c| !c.is_empty()),
                }
            },
        )
        .collect())
}

/// 例：data_type=character varying / udt=varchar / char_max=255 → "varchar(255)"
fn compose_full_type(data_type: &str, udt: &str, char_max: Option<i32>) -> String {
    let base = if udt.is_empty() { data_type } else { udt };
    if let Some(n) = char_max {
        format!("{base}({n})")
    } else {
        base.to_string()
    }
}

/// 含 BTREE/GIN/GIST/HASH/BRIN 等所有索引方法
pub async fn list_indexes(pool: &PgPool, schema: &str, table: &str) -> Result<Vec<Index>> {
    debug!(?schema, ?table, "list_indexes (postgres)");

    let rows: Vec<(String, bool, bool, Vec<String>)> = sqlx::query_as(
        r#"
        SELECT
            i.relname::text AS index_name,
            ix.indisunique AS is_unique,
            ix.indisprimary AS is_primary,
            array_agg(a.attname::text ORDER BY array_position(ix.indkey, a.attnum)) AS columns
        FROM pg_index ix
        JOIN pg_class i ON i.oid = ix.indexrelid
        JOIN pg_class t ON t.oid = ix.indrelid
        JOIN pg_namespace n ON n.oid = t.relnamespace
        JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = ANY(ix.indkey)
        WHERE n.nspname = $1 AND t.relname = $2
        GROUP BY i.relname, ix.indisunique, ix.indisprimary
        ORDER BY ix.indisprimary DESC, i.relname
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| map_postgres_error(&e))?;

    Ok(rows
        .into_iter()
        .map(|(name, unique, primary, columns)| Index {
            name,
            unique,
            primary,
            columns,
        })
        .collect())
}

pub async fn list_foreign_keys(
    pool: &PgPool,
    schema: &str,
    table: &str,
) -> Result<Vec<ForeignKey>> {
    debug!(?schema, ?table, "list_foreign_keys (postgres)");

    let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        r#"
        SELECT
            tc.constraint_name::text,
            kcu.column_name::text,
            ccu.table_schema::text AS ref_schema,
            ccu.table_name::text   AS ref_table,
            ccu.column_name::text  AS ref_column
        FROM information_schema.table_constraints tc
        JOIN information_schema.key_column_usage kcu
          ON tc.constraint_name = kcu.constraint_name
         AND tc.table_schema = kcu.table_schema
        JOIN information_schema.constraint_column_usage ccu
          ON ccu.constraint_name = tc.constraint_name
         AND ccu.table_schema = tc.table_schema
        WHERE tc.constraint_type = 'FOREIGN KEY'
          AND tc.table_schema = $1 AND tc.table_name = $2
        ORDER BY tc.constraint_name, kcu.ordinal_position
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| map_postgres_error(&e))?;

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
pub async fn ping(pool: &PgPool) -> Result<()> {
    let _: (i32,) = sqlx::query_as("SELECT 1")
        .fetch_one(pool)
        .await
        .map_err(|e| map_postgres_error(&e))?;
    Ok(())
}

/// `SHOW server_version`，形如 "13.5"
pub async fn server_version(pool: &PgPool) -> Result<String> {
    let (v,): (String,) = sqlx::query_as("SHOW server_version")
        .fetch_one(pool)
        .await
        .map_err(|e| map_postgres_error(&e))?;
    Ok(v)
}
