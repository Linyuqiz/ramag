//! SQL 文本工具：多语句切分 / LIMIT 注入 / 用户标记识别。方言中性

/// 多语句切分选项
#[derive(Debug, Clone, Copy)]
pub struct SplitOptions {
    /// 识别 PG dollar-quoted：`$$..$$` / `$tag$..$tag$`
    pub dollar_quoted: bool,
}

impl SplitOptions {
    pub fn mysql() -> Self {
        Self {
            dollar_quoted: false,
        }
    }

    pub fn postgres() -> Self {
        Self {
            dollar_quoted: true,
        }
    }
}

/// 按 `;` 切分，跳过字符串 / 行注释 / 块注释 / dollar-quoted 内的 `;`
pub fn split_statements(sql: &str, opts: SplitOptions) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\'' | b'"' | b'`' => {
                let quote = b;
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == quote {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            b'$' if opts.dollar_quoted => {
                if let Some(end) = scan_dollar_quoted(bytes, i) {
                    i = end;
                } else {
                    i += 1;
                }
            }
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
            }
            b';' => {
                let segment = sql[start..i].trim();
                if !segment.is_empty() {
                    out.push(segment.to_string());
                }
                start = i + 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    let tail = sql[start..].trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

/// 扫 dollar-quoted，返回闭合 tag 后的字节位置；非 dollar-quoted 返回 None。
/// 不处理嵌套（PG 也不允许同 tag 嵌套）。pub 给 UI 的「光标处取语句」用
pub fn scan_dollar_quoted(bytes: &[u8], start: usize) -> Option<usize> {
    debug_assert_eq!(bytes[start], b'$');
    let mut p = start + 1;
    while p < bytes.len() && (bytes[p].is_ascii_alphanumeric() || bytes[p] == b'_') {
        p += 1;
    }
    if p >= bytes.len() || bytes[p] != b'$' {
        return None;
    }
    let tag_end = p;
    let body_start = tag_end + 1;
    let tag = &bytes[start..=tag_end];

    let mut q = body_start;
    while q + tag.len() <= bytes.len() {
        if &bytes[q..q + tag.len()] == tag {
            return Some(q + tag.len());
        }
        q += 1;
    }
    None
}

/// 按首关键字粗判 SQL 是否返回结果集
pub fn is_query_returning_rows(sql: &str) -> bool {
    let upper: String = sql
        .chars()
        .skip_while(|c| c.is_whitespace())
        .take(8)
        .collect::<String>()
        .to_ascii_uppercase();

    upper.starts_with("SELECT")
        || upper.starts_with("SHOW")
        || upper.starts_with("DESC")
        || upper.starts_with("EXPLAIN")
        || upper.starts_with("WITH")
        || upper.starts_with("VALUES")
}

/// 取语句首关键字（大写）：跳过前导空白 / 行注释 / 块注释，取第一段连续字母。
/// 纯注释或非字母开头（如 `(SELECT ...)`）返回 None
pub fn first_keyword(stmt: &str) -> Option<String> {
    let bytes = stmt.as_bytes();
    let mut i = 0usize;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        // 行注释 --... 到行尾
        if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // 块注释 /* ... */
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        break;
    }
    let start = i;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        i += 1;
    }
    (i > start).then(|| stmt[start..i].to_ascii_uppercase())
}

/// 单条语句是否为写操作（黑名单）：生产模式只读保护用。
/// 首关键字命中写动词即写；CTE（WITH）/ EXPLAIN 进一步扫描语句体内的写动词
/// （覆盖 PG `WITH ... DELETE` 与 `EXPLAIN ANALYZE INSERT` 两个陷阱）
pub fn is_write_statement(stmt: &str) -> bool {
    // 首词即写动词。COPY/LOCK/VACUUM 等保守归写（生产只读不应执行）
    const WRITE_LEADING: &[&str] = &[
        "INSERT", "UPDATE", "DELETE", "REPLACE", "MERGE", "UPSERT", "TRUNCATE", "DROP", "CREATE",
        "ALTER", "RENAME", "GRANT", "REVOKE", "CALL", "EXEC", "EXECUTE", "LOAD", "COPY", "IMPORT",
        "REINDEX", "VACUUM", "CLUSTER", "REFRESH", "COMMENT", "LOCK",
    ];
    // CTE / EXPLAIN 语句体内若含这些动词则视为写
    const WRITE_INNER: &[&str] = &[
        "INSERT", "UPDATE", "DELETE", "MERGE", "REPLACE", "CREATE", "DROP", "ALTER", "TRUNCATE",
        "CALL",
    ];
    let Some(kw) = first_keyword(stmt) else {
        return false;
    };
    if WRITE_LEADING.contains(&kw.as_str()) {
        return true;
    }
    if kw == "WITH" || kw == "EXPLAIN" {
        let upper = stmt.to_ascii_uppercase();
        return WRITE_INNER.iter().any(|w| contains_word(&upper, w));
    }
    false
}

/// 仅对未带 LIMIT 的 SELECT/WITH 注入 ` LIMIT n`；其他语句返回 None
pub fn inject_limit_if_needed(stmt: &str, limit: Option<u32>) -> Option<String> {
    let n = limit?;
    if n == 0 {
        return None;
    }
    let prefix: String = stmt
        .chars()
        .take(8)
        .collect::<String>()
        .to_ascii_uppercase();
    if !(prefix.starts_with("SELECT") || prefix.starts_with("WITH")) {
        return None;
    }

    let mut tail_end = stmt.len();
    let bytes = stmt.as_bytes();
    while tail_end > 0 {
        let b = bytes[tail_end - 1];
        if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' || b == b';' {
            tail_end -= 1;
        } else {
            break;
        }
    }
    if tail_end == 0 {
        return None;
    }

    let core = &stmt[..tail_end];
    let scan_start = core.len().saturating_sub(64);
    let scan_str: String = core
        .char_indices()
        .skip_while(|(i, _)| *i < scan_start)
        .map(|(_, c)| c)
        .collect();
    let upper = scan_str.to_ascii_uppercase();
    if contains_word(&upper, "LIMIT") {
        return None;
    }

    let trailing_semicolon = stmt[tail_end..].contains(';');
    let mut out = String::with_capacity(core.len() + 16);
    out.push_str(core);
    out.push_str(&format!(" LIMIT {n}"));
    if trailing_semicolon {
        out.push(';');
    }
    Some(out)
}

/// 是否含 `-- ramag:no-limit` 跳过开关（大小写不敏感）
pub fn sql_has_no_limit_marker(sql: &str) -> bool {
    sql.lines().any(|line| {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("--") {
            rest.trim_start()
                .to_ascii_lowercase()
                .starts_with("ramag:no-limit")
        } else {
            false
        }
    })
}

/// 全词匹配（前后非字母数字下划线）
pub fn contains_word(haystack_upper: &str, word: &str) -> bool {
    let bytes = haystack_upper.as_bytes();
    let target = word.as_bytes();
    let mut i = 0;
    while i + target.len() <= bytes.len() {
        if &bytes[i..i + target.len()] == target {
            let before_ok = i == 0 || !is_word_byte(bytes[i - 1]);
            let after_ok =
                i + target.len() == bytes.len() || !is_word_byte(bytes[i + target.len()]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_single_no_semicolon() {
        assert_eq!(
            split_statements("SELECT 1", SplitOptions::mysql()),
            vec!["SELECT 1"]
        );
    }

    #[test]
    fn split_skips_semicolon_in_string() {
        let s = split_statements("SELECT 'a;b'; SELECT 2", SplitOptions::mysql());
        assert_eq!(s, vec!["SELECT 'a;b'", "SELECT 2"]);
    }

    #[test]
    fn split_skips_semicolon_in_line_comment() {
        let s = split_statements("SELECT 1 -- a;b\n; SELECT 2", SplitOptions::mysql());
        assert_eq!(s.len(), 2);
        assert_eq!(s[1], "SELECT 2");
    }

    #[test]
    fn split_postgres_dollar_quoted_basic() {
        let sql = "CREATE FUNCTION f() RETURNS int AS $$ BEGIN RETURN 1; END; $$ LANGUAGE plpgsql; SELECT 2";
        let s = split_statements(sql, SplitOptions::postgres());
        assert_eq!(s.len(), 2);
        assert!(s[0].contains("RETURN 1;"));
        assert_eq!(s[1], "SELECT 2");
    }

    #[test]
    fn split_postgres_tagged_dollar_quoted() {
        let sql = "DO $body$ BEGIN PERFORM 1; END; $body$; SELECT 3";
        let s = split_statements(sql, SplitOptions::postgres());
        assert_eq!(s.len(), 2);
        assert_eq!(s[1], "SELECT 3");
    }

    #[test]
    fn mysql_does_not_treat_dollar_as_quote() {
        let sql = "SELECT '$$abc$$'; SELECT 2";
        let s = split_statements(sql, SplitOptions::mysql());
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn inject_basic() {
        assert_eq!(
            inject_limit_if_needed("SELECT * FROM t", Some(500)).as_deref(),
            Some("SELECT * FROM t LIMIT 500")
        );
    }

    #[test]
    fn inject_skip_when_already_has_limit() {
        assert!(inject_limit_if_needed("SELECT * FROM t LIMIT 10", Some(500)).is_none());
    }

    #[test]
    fn detect_returning_rows() {
        assert!(is_query_returning_rows("SELECT 1"));
        assert!(is_query_returning_rows("VALUES (1, 2)"));
        assert!(!is_query_returning_rows("INSERT INTO t VALUES (1)"));
    }

    #[test]
    fn no_limit_marker() {
        assert!(sql_has_no_limit_marker("-- ramag:no-limit\nSELECT 1"));
        assert!(!sql_has_no_limit_marker("SELECT 'ramag:no-limit'"));
    }

    #[test]
    fn write_statement_dml_vs_select() {
        assert!(is_write_statement("INSERT INTO t VALUES (1)"));
        assert!(is_write_statement("  update t set x=1"));
        assert!(is_write_statement("DELETE FROM t"));
        assert!(!is_write_statement("SELECT 1"));
        assert!(!is_write_statement("select * from t"));
        assert!(!is_write_statement("SHOW TABLES"));
    }

    #[test]
    fn write_statement_ddl() {
        assert!(is_write_statement("DROP TABLE t"));
        assert!(is_write_statement("TRUNCATE TABLE t"));
        assert!(is_write_statement("create table t(id int)"));
        assert!(is_write_statement("ALTER TABLE t ADD c int"));
        assert!(is_write_statement("CALL proc()"));
    }

    #[test]
    fn write_statement_skips_leading_comment() {
        assert!(is_write_statement("-- danger\nDELETE FROM t"));
        assert!(is_write_statement("/* x */ DROP TABLE t"));
        assert!(!is_write_statement("-- just a select\nSELECT 1"));
    }

    #[test]
    fn write_statement_returning_is_write() {
        // PG：INSERT/UPDATE/DELETE ... RETURNING 会返回行但仍是写
        assert!(is_write_statement("INSERT INTO t VALUES (1) RETURNING id"));
        assert!(is_write_statement("UPDATE t SET x=1 RETURNING *"));
    }

    #[test]
    fn write_statement_cte_and_explain() {
        // 纯读 CTE / EXPLAIN 放行；CTE 内含写、EXPLAIN ANALYZE 真执行写则拦
        assert!(!is_write_statement("WITH x AS (SELECT 1) SELECT * FROM x"));
        assert!(is_write_statement(
            "WITH x AS (DELETE FROM t RETURNING *) SELECT * FROM x"
        ));
        assert!(!is_write_statement("EXPLAIN SELECT 1"));
        assert!(!is_write_statement("EXPLAIN ANALYZE SELECT 1"));
        assert!(is_write_statement(
            "EXPLAIN ANALYZE INSERT INTO t VALUES (1)"
        ));
    }

    #[test]
    fn write_statement_session_and_empty_are_readonly() {
        assert!(!is_write_statement("SET names utf8"));
        assert!(!is_write_statement("USE mydb"));
        assert!(!is_write_statement("BEGIN"));
        assert!(!is_write_statement(""));
        assert!(!is_write_statement("   "));
    }
}
