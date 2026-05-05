//! query_tab 共享的 SQL 工具函数
//!
//! - 自动 LIMIT 注入（`inject_limits` / `inject_limit_one`）
//! - SQL 多语句切分（`split_sql_statements` 转发到 `infra-sql-shared`）
//! - 光标处单语句提取（`extract_statement_at_cursor`，含 PG dollar-quoted 识别）
//! - 错误行号解析（`parse_mysql_error_line` 兼容 MySQL `at line N` / PG `LINE N:`）
//! - SQL → 短标题派生（`make_short_title`）
//! - 耗时格式化（`format_elapsed`）
//!
//! 全部纯函数，无 GPUI 依赖；测试在文件末尾

use std::time::Duration;

/// 默认自动 LIMIT 注入的上限
///
/// 提到 10000 配合表格虚拟化：服务端拉 1w 行 + 客户端 uniform_list 虚拟渲染都流畅；
/// 用户已写 LIMIT N 不会被覆盖（见 [`inject_limits`]）
/// 暴露给 connection_session 等同模块用，统一双击表名 / SHOW TABLE 等场景的 LIMIT
pub(crate) const AUTO_LIMIT: usize = 10_000;

/// 格式化运行中耗时：< 60s 显示 "X.Xs"，>= 60s 显示 "Mm Ss"
pub(super) fn format_elapsed(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs < 60.0 {
        format!("{secs:.1}s")
    } else {
        let m = (secs / 60.0) as u64;
        let s = secs as u64 % 60;
        format!("{m}m {s}s")
    }
}

/// 给"裸 SELECT / SHOW / DESC"自动注入 LIMIT，避免误把全表拉回来。
/// 多语句时按 `;` 切分逐条处理；PG 切分时识别 dollar-quoted 函数体内的 `;`。
/// 已经有 `LIMIT` / `WITH` / 非 SELECT 的语句保持原样。
pub(crate) fn inject_limits(
    sql: &str,
    max_rows: usize,
    driver: ramag_domain::entities::DriverKind,
) -> String {
    let stmts = split_sql_statements(sql, driver);
    if stmts.is_empty() {
        return sql.to_string();
    }
    let mut out = String::with_capacity(sql.len() + 16 * stmts.len());
    for (i, stmt) in stmts.iter().enumerate() {
        let s = inject_limit_one(stmt, max_rows);
        if i > 0 {
            out.push_str(";\n");
        }
        out.push_str(&s);
    }
    if sql.trim_end().ends_with(';') {
        out.push(';');
    }
    out
}

/// 单条语句 LIMIT 注入：仅 SELECT/WITH 类，且不含 LIMIT 时
fn inject_limit_one(stmt: &str, max_rows: usize) -> String {
    let trimmed = stmt.trim();
    if trimmed.is_empty() {
        return stmt.to_string();
    }
    let upper: String = trimmed
        .chars()
        .skip_while(|c| c.is_whitespace())
        .take(8)
        .collect::<String>()
        .to_ascii_uppercase();
    if !(upper.starts_with("SELECT") || upper.starts_with("WITH")) {
        return stmt.to_string();
    }
    let upper_full = trimmed.to_ascii_uppercase();
    if has_top_level_keyword(&upper_full, "LIMIT") {
        return stmt.to_string();
    }
    let body = trimmed.trim_end_matches(';').trim_end();
    format!("{body} LIMIT {max_rows}")
}

/// 检测 SQL 中是否有顶层（不在括号子查询里）的关键字（如 LIMIT）
/// 简化处理：跳过字符串/反引号/注释，扫描 keyword 边界
fn has_top_level_keyword(sql_upper: &str, keyword: &str) -> bool {
    let bytes = sql_upper.as_bytes();
    let kw = keyword.as_bytes();
    let mut depth: i32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\'' | b'"' | b'`' => {
                let q = b;
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == q {
                        i += 1;
                        break;
                    }
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
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                i += 1;
            }
            _ if depth == 0 && i + kw.len() <= bytes.len() && &bytes[i..i + kw.len()] == kw => {
                let prev_ok = i == 0 || !is_ident(bytes[i - 1]);
                let next_idx = i + kw.len();
                let next_ok = next_idx >= bytes.len() || !is_ident(bytes[next_idx]);
                if prev_ok && next_ok {
                    return true;
                }
                i += 1;
            }
            _ => i += 1,
        }
    }
    false
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// 多语句切分：复用 sql-shared 的实现，按 driver 选择是否识别 PG dollar-quoted
pub(super) fn split_sql_statements(
    sql: &str,
    driver: ramag_domain::entities::DriverKind,
) -> Vec<String> {
    let opts = match driver {
        ramag_domain::entities::DriverKind::Postgres => {
            ramag_infra_sql_shared::sql::SplitOptions::postgres()
        }
        _ => ramag_infra_sql_shared::sql::SplitOptions::mysql(),
    };
    ramag_infra_sql_shared::sql::split_statements(sql, opts)
}

/// 从错误消息里提取行号（兼容 MySQL / PostgreSQL 两种格式）
///
/// - MySQL：消息含 `... at line N`
/// - PostgreSQL：消息含 `LINE N:`（PG 错误的次行标准格式）
pub(crate) fn parse_mysql_error_line(msg: &str) -> Option<usize> {
    if let Some(idx) = msg.find(" at line ") {
        let tail = &msg[idx + " at line ".len()..];
        let num: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = num.parse::<usize>() {
            return Some(n);
        }
    }
    if let Some(idx) = msg.find("LINE ") {
        let tail = &msg[idx + "LINE ".len()..];
        let num: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = num.parse::<usize>() {
            return Some(n);
        }
    }
    None
}

/// 提取光标所在的那条 SQL 语句（按 `;` 切分）
///
/// 切分时跳过下列结构里的 `;`：
/// - 单引号 / 双引号 / 反引号 字符串（含 `\\` 转义）
/// - `--` 行注释 / `/* */` 块注释
/// - PG dollar-quoted 函数体（`$$ ... $$` / `$tag$ ... $tag$`，仅 driver=Postgres）
///
/// `cursor` 是 UTF-8 byte offset；越界时按最后一条处理。
pub(super) fn extract_statement_at_cursor(
    sql: &str,
    cursor: usize,
    driver: Option<ramag_domain::entities::DriverKind>,
) -> &str {
    let pg = matches!(driver, Some(ramag_domain::entities::DriverKind::Postgres));
    let bytes = sql.as_bytes();
    let cursor = cursor.min(bytes.len());
    let mut splits: Vec<usize> = Vec::new();
    let mut i = 0;
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
            b'$' if pg => {
                if let Some(end) = ramag_infra_sql_shared::sql::scan_dollar_quoted(bytes, i) {
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
                i += 2;
            }
            b';' => {
                splits.push(i);
                i += 1;
            }
            _ => i += 1,
        }
    }

    let mut start = 0;
    for &sp in &splits {
        if sp >= cursor {
            return safe_str_slice(sql, start, sp);
        }
        start = sp + 1;
    }
    safe_str_slice(sql, start, bytes.len())
}

fn safe_str_slice(sql: &str, mut start: usize, mut end: usize) -> &str {
    let bytes = sql.as_bytes();
    while start < bytes.len() && !sql.is_char_boundary(start) {
        start += 1;
    }
    while end > 0 && !sql.is_char_boundary(end) {
        end -= 1;
    }
    if end < start {
        return "";
    }
    &sql[start..end]
}

/// 从 SQL 派生短标题：取首条非空行前 28 个字符（按字符计，不按字节）
pub(super) fn make_short_title(sql: &str) -> String {
    const MAX: usize = 28;
    let first_line = sql
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if first_line.chars().count() > MAX {
        let prefix: String = first_line.chars().take(MAX).collect();
        format!("{prefix}…")
    } else {
        first_line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_title_truncate() {
        assert_eq!(make_short_title("SELECT 1"), "SELECT 1");
        assert_eq!(
            make_short_title("SELECT * FROM very_long_table_name_here"),
            "SELECT * FROM very_long_tabl…"
        );
    }

    #[test]
    fn short_title_skips_blank_lines() {
        let sql = "\n\n  -- comment\nSELECT 1";
        assert_eq!(make_short_title(sql), "-- comment");
    }

    #[test]
    fn short_title_empty() {
        assert_eq!(make_short_title(""), "");
        assert_eq!(make_short_title("   "), "");
    }

    /// EXPLAIN 包装策略：模拟 actions.rs::handle_explain 的 SQL 处理
    fn wrap_explain(sql: &str) -> String {
        let trimmed = sql.trim().trim_end_matches(';').trim().to_string();
        if trimmed.is_empty() {
            return String::new();
        }
        let upper = trimmed.to_ascii_uppercase();
        if upper.starts_with("EXPLAIN ") || upper == "EXPLAIN" {
            trimmed
        } else {
            format!("EXPLAIN {trimmed}")
        }
    }

    #[test]
    fn parse_mysql_line() {
        assert_eq!(
            parse_mysql_error_line("You have an error in your SQL syntax... near 'foo' at line 3"),
            Some(3)
        );
        assert_eq!(parse_mysql_error_line("connection refused"), None);
        assert_eq!(parse_mysql_error_line("error at line 12"), Some(12));
    }

    #[test]
    fn parse_postgres_line() {
        assert_eq!(
            parse_mysql_error_line("syntax error at end of input\nLINE 5: SELECT *"),
            Some(5)
        );
        assert_eq!(parse_mysql_error_line("LINE 1: SELECT * FORM t"), Some(1));
    }

    use ramag_domain::entities::DriverKind;

    #[test]
    fn inject_limit_plain_select() {
        let s = inject_limits("SELECT * FROM t", 1000, DriverKind::Mysql);
        assert_eq!(s, "SELECT * FROM t LIMIT 1000");
    }

    #[test]
    fn inject_limit_skips_existing_limit() {
        let s = inject_limits("SELECT * FROM t LIMIT 10", 1000, DriverKind::Mysql);
        assert_eq!(s, "SELECT * FROM t LIMIT 10");
        let s = inject_limits("select * from t limit 10", 1000, DriverKind::Mysql);
        assert_eq!(s, "select * from t limit 10");
    }

    #[test]
    fn inject_limit_skips_non_select() {
        assert_eq!(
            inject_limits("UPDATE t SET a=1", 1000, DriverKind::Mysql),
            "UPDATE t SET a=1"
        );
        assert_eq!(
            inject_limits("SHOW TABLES", 1000, DriverKind::Mysql),
            "SHOW TABLES"
        );
    }

    #[test]
    fn inject_limit_keeps_subquery_limit_alone() {
        let s = inject_limits(
            "SELECT * FROM (SELECT * FROM t LIMIT 10) x",
            1000,
            DriverKind::Mysql,
        );
        assert!(s.ends_with("LIMIT 1000"));
    }

    #[test]
    fn inject_limit_strips_trailing_semicolon() {
        let s = inject_limits("SELECT * FROM t;", 1000, DriverKind::Mysql);
        assert_eq!(s, "SELECT * FROM t LIMIT 1000;");
    }

    #[test]
    fn inject_limit_postgres_dollar_quoted_function_body_not_split() {
        let sql = "CREATE FUNCTION f() RETURNS int AS $$ BEGIN RETURN 1; END; $$ LANGUAGE plpgsql";
        let s = inject_limits(sql, 1000, DriverKind::Postgres);
        assert_eq!(s, sql);
    }

    #[test]
    fn extract_stmt_postgres_dollar_quoted_keeps_function_body_intact() {
        let sql = "CREATE FUNCTION f() RETURNS int AS $$ BEGIN RETURN 1; END; $$ LANGUAGE plpgsql; SELECT 2";
        let pg = Some(DriverKind::Postgres);
        let stmt = extract_statement_at_cursor(sql, 45, pg);
        assert!(stmt.contains("CREATE FUNCTION"));
        assert!(stmt.contains("END"));
    }

    #[test]
    fn extract_stmt_postgres_dollar_quoted_picks_next_statement() {
        let sql = "CREATE FUNCTION f() RETURNS int AS $$ BEGIN RETURN 1; END; $$ LANGUAGE plpgsql; SELECT 2";
        let pg = Some(DriverKind::Postgres);
        let stmt = extract_statement_at_cursor(sql, sql.len() - 1, pg).trim();
        assert_eq!(stmt, "SELECT 2");
    }

    #[test]
    fn extract_stmt_single() {
        assert_eq!(
            extract_statement_at_cursor("SELECT 1", 5, None).trim(),
            "SELECT 1"
        );
    }

    #[test]
    fn extract_stmt_multi_picks_by_cursor() {
        let sql = "SELECT 1; SELECT 2; SELECT 3";
        assert_eq!(extract_statement_at_cursor(sql, 3, None).trim(), "SELECT 1");
        assert_eq!(
            extract_statement_at_cursor(sql, 12, None).trim(),
            "SELECT 2"
        );
        assert_eq!(
            extract_statement_at_cursor(sql, 25, None).trim(),
            "SELECT 3"
        );
    }

    #[test]
    fn extract_stmt_ignores_semicolon_in_string() {
        let sql = "SELECT 'a;b'; SELECT 2";
        assert_eq!(
            extract_statement_at_cursor(sql, 5, None).trim(),
            "SELECT 'a;b'"
        );
        assert_eq!(
            extract_statement_at_cursor(sql, 18, None).trim(),
            "SELECT 2"
        );
    }

    #[test]
    fn extract_stmt_ignores_semicolon_in_comment() {
        let sql = "SELECT 1 -- comment ;\n; SELECT 2";
        let first = extract_statement_at_cursor(sql, 5, None);
        assert!(first.contains("SELECT 1"));
        assert!(!first.contains("SELECT 2"));
        assert_eq!(
            extract_statement_at_cursor(sql, 26, None).trim(),
            "SELECT 2"
        );
    }

    #[test]
    fn explain_wraps_plain_select() {
        assert_eq!(wrap_explain("SELECT 1"), "EXPLAIN SELECT 1");
        assert_eq!(
            wrap_explain("SELECT * FROM t WHERE id=1;"),
            "EXPLAIN SELECT * FROM t WHERE id=1"
        );
    }

    #[test]
    fn explain_does_not_double_wrap() {
        assert_eq!(wrap_explain("EXPLAIN SELECT 1"), "EXPLAIN SELECT 1");
        assert_eq!(wrap_explain("explain  SELECT 1"), "explain  SELECT 1");
    }

    #[test]
    fn explain_strips_trailing_semicolons() {
        assert_eq!(wrap_explain("SELECT 1;;;"), "EXPLAIN SELECT 1");
    }

    #[test]
    fn sqlformat_works() {
        let opts = sqlformat::FormatOptions {
            indent: sqlformat::Indent::Spaces(2),
            uppercase: Some(true),
            lines_between_queries: 1,
            ignore_case_convert: None,
        };
        let formatted = sqlformat::format(
            "select id,name from users where id=1 order by name",
            &sqlformat::QueryParams::None,
            &opts,
        );
        assert!(formatted.contains("SELECT"));
        assert!(formatted.contains("FROM"));
        assert!(formatted.contains("WHERE"));
        assert!(formatted.lines().count() >= 3);
    }
}
