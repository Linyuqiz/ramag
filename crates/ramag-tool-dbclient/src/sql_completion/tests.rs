use super::*;

#[test]
fn keywords_uppercase_only() {
    for kw in SQL_KEYWORDS {
        assert!(kw.chars().all(|c| !c.is_lowercase()), "keyword {kw} 含小写");
    }
}

#[test]
fn detect_table_context() {
    assert_eq!(detect_context("SELECT * FROM "), SqlContext::Table);
    assert_eq!(
        detect_context("SELECT a FROM users JOIN "),
        SqlContext::Table
    );
    assert_eq!(detect_context("UPDATE "), SqlContext::Table);
    assert_eq!(detect_context("INSERT INTO "), SqlContext::Table);
}

#[test]
fn detect_column_context() {
    // SELECT 后到 FROM 之前
    assert_eq!(detect_context("SELECT "), SqlContext::Column);
    // WHERE / AND / OR / ON / HAVING
    assert_eq!(
        detect_context("SELECT * FROM users WHERE "),
        SqlContext::Column
    );
    assert_eq!(
        detect_context("SELECT * FROM users WHERE id = 1 AND "),
        SqlContext::Column
    );
    assert_eq!(
        detect_context("SELECT a FROM x JOIN y ON "),
        SqlContext::Column
    );
    // ORDER BY / GROUP BY 多词
    assert_eq!(
        detect_context("SELECT * FROM x ORDER BY "),
        SqlContext::Column
    );
    assert_eq!(
        detect_context("SELECT * FROM x GROUP BY "),
        SqlContext::Column
    );
    // UPDATE ... SET
    assert_eq!(detect_context("UPDATE x SET "), SqlContext::Column);
}

#[test]
fn detect_other_context() {
    assert_eq!(detect_context(""), SqlContext::Other);
    assert_eq!(detect_context("LIMIT "), SqlContext::Other);
}

#[test]
fn extract_tables_basic() {
    assert_eq!(
        extract_tables_in_use("SELECT * FROM users"),
        vec!["users".to_string()]
    );
    assert_eq!(
        extract_tables_in_use("SELECT a FROM users JOIN orders"),
        vec!["users".to_string(), "orders".to_string()]
    );
    // 反引号 + schema.table 形式
    assert_eq!(
        extract_tables_in_use("SELECT * FROM `db`.`users`"),
        vec!["users".to_string()]
    );
}

#[test]
fn cache_default_schema_first() {
    let mut c = SchemaCache {
        default_schema: Some("midas".to_string()),
        ..Default::default()
    };
    c.tables.insert(
        "midas".to_string(),
        vec!["users".to_string(), "orders".to_string()],
    );
    c.tables
        .insert("logs".to_string(), vec!["events".to_string()]);
    let all = c.all_tables();
    // 默认 schema 的表必须排在前面
    assert!(all.iter().position(|x| x == "users") < all.iter().position(|x| x == "events"));
}

#[test]
fn extract_tables_with_schema_simple() {
    let v = extract_tables_with_schema("SELECT * FROM mydb.users");
    assert_eq!(v, vec![(Some("mydb".to_string()), "users".to_string())]);
}

#[test]
fn extract_tables_with_schema_no_schema() {
    let v = extract_tables_with_schema("SELECT * FROM users");
    assert_eq!(v, vec![(None, "users".to_string())]);
}

#[test]
fn extract_tables_with_schema_join_mixed() {
    let v = extract_tables_with_schema("SELECT * FROM a.users JOIN orders");
    assert_eq!(
        v,
        vec![
            (Some("a".to_string()), "users".to_string()),
            (None, "orders".to_string()),
        ]
    );
}

#[test]
fn extract_tables_with_schema_catalog_three_parts() {
    // catalog.schema.table 形式（如 SQL Server）：取后两段
    let v = extract_tables_with_schema("SELECT * FROM cat.midas.users");
    assert_eq!(v, vec![(Some("midas".to_string()), "users".to_string())]);
}

#[test]
fn extract_tables_update_into() {
    let v_upd = extract_tables_in_use("UPDATE users SET a=1");
    assert_eq!(v_upd, vec!["users".to_string()]);
    let v_ins = extract_tables_in_use("INSERT INTO orders VALUES (1)");
    assert_eq!(v_ins, vec!["orders".to_string()]);
}

#[test]
fn extract_tables_strip_quotes() {
    // PostgreSQL 双引号、MySQL 反引号都得脱掉
    let v_pg = extract_tables_with_schema("SELECT * FROM \"public\".\"users\"");
    assert_eq!(
        v_pg,
        vec![(Some("public".to_string()), "users".to_string())]
    );
    let v_mysql = extract_tables_with_schema("SELECT * FROM `db`.`tbl`");
    assert_eq!(v_mysql, vec![(Some("db".to_string()), "tbl".to_string())]);
}

#[test]
fn phrase_prefix_multiword() {
    // 第一个词已敲完、正敲第二个词：取整段短语（单词 prefix 只剩 "T"，匹配不到 DROP TABLE）
    assert_eq!(phrase_prefix("DROP T", 6), "DROP T");
    assert_eq!(
        phrase_prefix("ALTER TABLE foo ADD CO", 22),
        "ALTER TABLE foo ADD CO"
    );
    // 分号 / 逗号 / 括号 / 换行都断开短语，并 trim 前导空格
    assert_eq!(phrase_prefix("SELECT 1; DROP TA", 17), "DROP TA");
    assert_eq!(phrase_prefix("a,\n  CREATE D", 13), "CREATE D");
    // 空输入 + offset 越界（自动收敛到末尾）
    assert_eq!(phrase_prefix("", 0), "");
    assert_eq!(phrase_prefix("USE", 99), "USE");
}

#[test]
fn extract_table_refs_aliases() {
    use super::alias::extract_table_refs;
    // 裸别名 `users u`
    let r = extract_table_refs("SELECT u.id FROM users u");
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].table.as_str(), "users");
    assert_eq!(r[0].alias.as_deref(), Some("u"));
    assert_eq!(r[0].schema, None);
    // AS 别名
    let r = extract_table_refs("SELECT * FROM orders AS o WHERE o.id = 1");
    assert_eq!(r[0].alias.as_deref(), Some("o"));
    // 表名后是子句关键字 → 无别名（不把 WHERE / ORDER 误当别名）
    assert_eq!(
        extract_table_refs("SELECT * FROM users WHERE id=1")[0].alias,
        None
    );
    assert_eq!(
        extract_table_refs("SELECT * FROM users ORDER BY id")[0].alias,
        None
    );
    // JOIN + schema.table + 两处别名
    let r = extract_table_refs("SELECT * FROM db.users u JOIN db.orders o ON u.id = o.uid");
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].schema.as_deref(), Some("db"));
    assert_eq!(r[0].table.as_str(), "users");
    assert_eq!(r[0].alias.as_deref(), Some("u"));
    assert_eq!(r[1].table.as_str(), "orders");
    assert_eq!(r[1].alias.as_deref(), Some("o"));
    // UPDATE 带别名
    let r = extract_table_refs("UPDATE accounts a SET bal = 0");
    assert_eq!(r[0].alias.as_deref(), Some("a"));
    // 已知局限：逗号分隔的非首表（无 FROM/JOIN 前导）暂不解析，只得首表
    let r = extract_table_refs("SELECT * FROM users u, orders o");
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].alias.as_deref(), Some("u"));
}
