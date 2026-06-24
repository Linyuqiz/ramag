//! 常用 SQL 示例模板：按方言生成，表名优先用当前选中的表

use ramag_domain::entities::DriverKind;

/// 生成（菜单标签, SQL 模板）列表。`table` 为空时用占位名
pub(crate) fn sql_examples(driver: DriverKind, table: &str) -> Vec<(&'static str, String)> {
    let t = if table.trim().is_empty() {
        "your_table"
    } else {
        table
    };
    let mut out = vec![
        (
            "条件查询",
            format!("SELECT *\nFROM {t}\nWHERE id > 0\nORDER BY id DESC\nLIMIT 100;"),
        ),
        (
            "分组统计",
            format!(
                "SELECT your_column, COUNT(*) AS cnt\nFROM {t}\nGROUP BY your_column\nORDER BY cnt DESC;"
            ),
        ),
        (
            "两表关联",
            format!(
                "SELECT a.*, b.id AS b_id\nFROM {t} a\nJOIN other_table b ON a.id = b.ref_id\nLIMIT 100;"
            ),
        ),
        (
            "插入一行",
            format!("INSERT INTO {t} (col1, col2)\nVALUES ('v1', 'v2');"),
        ),
        (
            "条件更新",
            format!("UPDATE {t}\nSET col1 = 'new_value'\nWHERE id = 1;"),
        ),
        ("条件删除", format!("DELETE FROM {t}\nWHERE id = 1;")),
    ];
    match driver {
        DriverKind::Postgres => {
            out.push((
                "建表",
                "CREATE TABLE new_table (\n  id BIGSERIAL PRIMARY KEY,\n  name TEXT NOT NULL,\n  created_at TIMESTAMPTZ DEFAULT now()\n);"
                    .to_string(),
            ));
            out.push((
                "正在执行的查询",
                "SELECT pid, state, query, now() - query_start AS duration\nFROM pg_stat_activity\nWHERE state <> 'idle'\nORDER BY duration DESC;"
                    .to_string(),
            ));
            out.push((
                "各表大小",
                "SELECT relname AS table_name,\n  pg_size_pretty(pg_total_relation_size(relid)) AS total_size\nFROM pg_catalog.pg_statio_user_tables\nORDER BY pg_total_relation_size(relid) DESC;"
                    .to_string(),
            ));
        }
        _ => {
            out.push((
                "建表",
                "CREATE TABLE new_table (\n  id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,\n  name VARCHAR(255) NOT NULL,\n  created_at DATETIME DEFAULT CURRENT_TIMESTAMP\n);"
                    .to_string(),
            ));
            out.push(("正在执行的查询", "SHOW FULL PROCESSLIST;".to_string()));
            out.push((
                "各表行数与大小",
                "SELECT TABLE_NAME, TABLE_ROWS,\n  ROUND((DATA_LENGTH + INDEX_LENGTH) / 1024 / 1024, 1) AS size_mb\nFROM information_schema.TABLES\nWHERE TABLE_SCHEMA = DATABASE()\nORDER BY size_mb DESC;"
                    .to_string(),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_selected_table_name() {
        let items = sql_examples(DriverKind::Mysql, "orders");
        assert!(items.iter().any(|(_, sql)| sql.contains("FROM orders")));
    }

    #[test]
    fn falls_back_to_placeholder() {
        let items = sql_examples(DriverKind::Mysql, "  ");
        assert!(items.iter().any(|(_, sql)| sql.contains("your_table")));
    }

    #[test]
    fn dialect_specific_items() {
        let my = sql_examples(DriverKind::Mysql, "t");
        assert!(my.iter().any(|(_, sql)| sql.contains("PROCESSLIST")));
        let pg = sql_examples(DriverKind::Postgres, "t");
        assert!(pg.iter().any(|(_, sql)| sql.contains("pg_stat_activity")));
        assert_eq!(my.len(), pg.len());
    }
}
