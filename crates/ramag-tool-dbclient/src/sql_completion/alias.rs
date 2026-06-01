//! 表别名解析：从 SQL 提取 FROM/JOIN/INTO/UPDATE 后的 `[schema.]table [AS] alias`，
//! 供点号限定列补全（`u.col` / `users.col` / `mydb.tbl`）用

/// 一处表引用：schema 可选、table 必有、alias 可选
#[derive(Debug, PartialEq)]
pub(super) struct TableRef {
    pub schema: Option<String>,
    pub table: String,
    pub alias: Option<String>,
}

/// 表名后紧跟这些词说明没有别名（是子句关键字而非别名）
const NON_ALIAS: &[&str] = &[
    "ON",
    "USING",
    "WHERE",
    "JOIN",
    "INNER",
    "LEFT",
    "RIGHT",
    "OUTER",
    "FULL",
    "CROSS",
    "SET",
    "GROUP",
    "ORDER",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "UNION",
    "INTERSECT",
    "EXCEPT",
    "VALUES",
    "RETURNING",
];

/// 提取所有表引用（含别名）。例：`FROM users u JOIN orders AS o`
/// → [{users, alias:u}, {orders, alias:o}]
///
/// 局限：仅解析 FROM/JOIN/INTO/UPDATE 紧跟的表；逗号分隔的非首表（`FROM a, b`）暂不解析
pub(super) fn extract_table_refs(sql: &str) -> Vec<TableRef> {
    let orig: Vec<&str> = sql.split_ascii_whitespace().collect();
    let mut refs = Vec::new();
    let mut i = 0;
    while i < orig.len() {
        if matches!(kw_of(orig[i]).as_str(), "FROM" | "JOIN" | "INTO" | "UPDATE")
            && i + 1 < orig.len()
        {
            let table_tok = orig[i + 1];
            let (schema, table) = parse_qualified(table_tok);
            if !table.is_empty() {
                let alias = parse_alias(&orig, i, table_tok.ends_with(','));
                refs.push(TableRef {
                    schema,
                    table,
                    alias,
                });
            }
        }
        i += 1;
    }
    refs
}

/// 表名后的别名：`AS x` 取 x；裸标识符且非子句关键字取之；逗号粘连 / 子句词 → None
fn parse_alias(orig: &[&str], kw_idx: usize, comma_after: bool) -> Option<String> {
    if comma_after || kw_idx + 2 >= orig.len() {
        return None;
    }
    let next = kw_of(orig[kw_idx + 2]);
    if next == "AS" {
        orig.get(kw_idx + 3).and_then(|t| clean_ident(t))
    } else if NON_ALIAS.contains(&next.as_str()) {
        None
    } else {
        clean_ident(orig[kw_idx + 2])
    }
}

/// token 转大写并去尾部标点（如 `users,` → `USERS`）
fn kw_of(tok: &str) -> String {
    tok.to_ascii_uppercase()
        .trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .to_string()
}

/// 拆 `schema.table` / `catalog.schema.table` / `table`，去引号/逗号，取末两段
fn parse_qualified(tok: &str) -> (Option<String>, String) {
    let cleaned: String = tok
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
        .collect();
    let parts: Vec<&str> = cleaned.split('.').filter(|s| !s.is_empty()).collect();
    match parts.as_slice() {
        [t] => (None, (*t).to_string()),
        [s, t] => (Some((*s).to_string()), (*t).to_string()),
        [_, s, t] => (Some((*s).to_string()), (*t).to_string()),
        _ => (None, String::new()),
    }
}

/// 清出纯标识符（去引号/逗号等非法字符），空则 None
fn clean_ident(tok: &str) -> Option<String> {
    let s: String = tok
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if s.is_empty() { None } else { Some(s) }
}
