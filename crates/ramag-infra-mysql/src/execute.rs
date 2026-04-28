//! 执行 SQL 查询并构造 QueryResult
//!
//! 区分 SELECT 类查询（返回结果集）vs 非 SELECT 类（返回 affected_rows）。

use std::sync::atomic::Ordering;
use std::time::Instant;

use ramag_domain::entities::{Query, QueryResult, Row, Warning};
use ramag_domain::error::Result;
use ramag_domain::traits::CancelHandle;
use sqlx::mysql::MySqlRow;
use sqlx::pool::PoolConnection;
use sqlx::{Column as _, Executor, MySql, MySqlPool, Row as _, TypeInfo as _};
use tracing::{debug, info, warn};

use crate::errors::map_sqlx_error;
use crate::types::decode_row;

/// 在指定连接池上执行 query
///
/// - 若是 SELECT 类，返回行数据 + 列名
/// - 若是 INSERT/UPDATE/DELETE 等，返回 affected_rows
/// - 若 query 带 `default_schema`，则在 acquire 出来的同一连接上先 `USE \`db\`` 再跑 SQL
///   （MySQL USE 是 session 级，必须与后续 SQL 共享同一物理连接）
/// - 多语句（按 `;` 分隔）逐条执行；中间任意条失败立即返回 Err；
///   返回值是「最后一条」的 QueryResult；DML 类的 affected_rows 累加
/// - 若 `handle` 为 Some：acquire 后立即 `SELECT CONNECTION_ID()` 拿当前连接 thread id
///   写入 handle，让外部 cancel 路径能发 `KILL QUERY <id>`
pub async fn execute(
    pool: &MySqlPool,
    query: &Query,
    handle: Option<CancelHandle>,
) -> Result<QueryResult> {
    let start = Instant::now();

    let mut conn: PoolConnection<MySql> = pool.acquire().await.map_err(map_sqlx_error)?;

    // 把后端 thread id 写入 handle（仅当 handle Some）；失败不阻塞主查询，仅 warn
    if let Some(h) = &handle {
        match sqlx::query_as::<_, (u64,)>("SELECT CONNECTION_ID()")
            .fetch_one(&mut *conn)
            .await
        {
            Ok((tid,)) => h.store(tid, Ordering::SeqCst),
            Err(e) => warn!(error = %e, "failed to fetch CONNECTION_ID for cancel"),
        }
    }

    if let Some(schema) = query.default_schema.as_deref().filter(|s| !s.is_empty()) {
        let use_sql = format!("USE `{}`", schema.replace('`', "``"));
        debug!(?use_sql, "switching default schema before query");
        conn.execute(use_sql.as_str())
            .await
            .map_err(map_sqlx_error)?;
    }

    // 切分多条语句；单条 SQL 时 statements.len() == 1
    let statements = split_statements(&query.sql);
    if statements.is_empty() {
        return Ok(QueryResult {
            columns: Vec::new(),
            column_types: Vec::new(),
            rows: Vec::new(),
            affected_rows: 0,
            elapsed_ms: start.elapsed().as_millis() as u64,
            warnings: Vec::new(),
        });
    }
    let last_idx = statements.len() - 1;
    let mut total_affected: u64 = 0;
    // 多语句执行的 warnings 聚合：每条 statement 后立即拉取（MySQL 下条语句会清空 warning buffer）
    let mut accumulated_warnings: Vec<Warning> = Vec::new();
    let mut last_result = QueryResult {
        columns: Vec::new(),
        column_types: Vec::new(),
        rows: Vec::new(),
        affected_rows: 0,
        elapsed_ms: 0,
        warnings: Vec::new(),
    };

    // 用户级跳过开关：SQL 含 `-- ramag:no-limit` 注释时整段都不注入
    // （写在任意位置生效，不区分语句；此开关比 query.auto_limit 优先级高）
    let user_disabled_limit = sql_has_no_limit_marker(&query.sql);

    for (i, stmt) in statements.iter().enumerate() {
        let trimmed = stmt.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let is_select = is_query_returning_rows(trimmed);
        // 仅对 SELECT/WITH 起头的"原生 SELECT"注入 LIMIT；SHOW/DESC/EXPLAIN 不注入
        let injected: Option<String> = if is_select && !user_disabled_limit {
            inject_limit_if_needed(trimmed, query.auto_limit)
        } else {
            None
        };
        let effective_sql: &str = injected.as_deref().unwrap_or(stmt.as_str());
        let r = if is_select {
            execute_query(&mut conn, effective_sql).await?
        } else {
            execute_dml(&mut conn, effective_sql).await?
        };
        if !is_select {
            total_affected = total_affected.saturating_add(r.affected_rows);
        }
        // 立即抓本条语句产生的 warnings：MySQL 下条语句执行会清 warning buffer
        // 多语句时累积所有条的 warnings；单语句只有这一次
        let stmt_warnings = fetch_warnings(&mut conn).await;
        if !stmt_warnings.is_empty() {
            accumulated_warnings.extend(stmt_warnings);
        }
        if i == last_idx {
            last_result = r;
        }
    }
    // DML/DDL 链最后一条仍是 DML：用累加的 affected_rows 替换
    if last_result.rows.is_empty() && last_result.columns.is_empty() {
        last_result.affected_rows = total_affected;
    }

    let elapsed_ms = start.elapsed().as_millis() as u64;
    let warnings_count = accumulated_warnings.len();
    info!(
        elapsed_ms,
        rows = last_result.rows.len(),
        affected = last_result.affected_rows,
        statements = statements.len(),
        warnings = warnings_count,
        "query executed"
    );

    Ok(QueryResult {
        elapsed_ms,
        warnings: accumulated_warnings,
        ..last_result
    })
}

/// 按 `;` 切分多条 SQL，跳过 字符串 / `--` 行注释 / `/* */` 块注释 内的 `;`
/// 单条且无 `;` 的 SQL 切出来仍是单元素 vec
pub(crate) fn split_statements(sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut start = 0;
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

/// 判断 SQL 是否返回结果集
fn is_query_returning_rows(sql: &str) -> bool {
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
}

/// 仅返回需要追加 LIMIT 的"原生 SELECT/WITH" 语句
///
/// 设计意图：保持极简、稳定、可预期，不引入 SQL parser
/// - 仅当首单词是 SELECT 或 WITH 时考虑注入（SHOW/DESC/EXPLAIN/SET 等都跳过）
/// - 检测末尾是否已含 LIMIT：去尾分号空白后取末尾约 64 字符大写化扫描 ` LIMIT ` 字面
///   误报代价低（已含会跳过注入）；漏报代价高（注入到已有的 SQL 末尾会语法错）
///   所以倾向"宁可不注入也别错注入"
/// - limit=None 时返回 None（不注入）；limit=Some(0) 也不注入（语义无意义）
pub(crate) fn inject_limit_if_needed(stmt: &str, limit: Option<u32>) -> Option<String> {
    let n = limit?;
    if n == 0 {
        return None;
    }
    // 必须以 SELECT 或 WITH 开头（已剔除前导空白由调用方传入）
    let prefix: String = stmt
        .chars()
        .take(8)
        .collect::<String>()
        .to_ascii_uppercase();
    if !(prefix.starts_with("SELECT") || prefix.starts_with("WITH")) {
        return None;
    }

    // 去尾部分号 / 空白 / 尾部块注释
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

    // 取末尾最多 64 字符做大写化扫描（避免对完整 SQL 全文扫描），
    // 64 足够容纳 LIMIT N OFFSET M 这样的尾部子句
    let core = &stmt[..tail_end];
    let scan_start = core.len().saturating_sub(64);
    // 切到字符边界（防止多字节字符切坏，按字符走）
    let scan_str: String = core
        .char_indices()
        .skip_while(|(i, _)| *i < scan_start)
        .map(|(_, c)| c)
        .collect();
    let upper = scan_str.to_ascii_uppercase();
    // 匹配前后均为非字母数字以避开 column 名 limit；用空格 + 字符判断
    if contains_word(&upper, "LIMIT") {
        return None;
    }

    // 拼接：保留尾部分号（如有）以兼容多语句切分；注入到 ; 之前
    let trailing_semicolon = stmt[tail_end..].contains(';');
    let mut out = String::with_capacity(core.len() + 16);
    out.push_str(core);
    out.push_str(&format!(" LIMIT {n}"));
    if trailing_semicolon {
        out.push(';');
    }
    Some(out)
}

/// 在大写化字符串里寻找完整单词 `word`（前后是非字母数字或边界）
/// 用于避免把 column 名叫 `limit` 当作子句关键字
fn contains_word(haystack_upper: &str, word: &str) -> bool {
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

/// 检测 SQL 中是否含 `-- ramag:no-limit` 行注释（用户级跳过开关）
/// 大小写不敏感；位置任意
fn sql_has_no_limit_marker(sql: &str) -> bool {
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

async fn execute_query(conn: &mut PoolConnection<MySql>, sql: &str) -> Result<QueryResult> {
    debug!(?sql, "executing SELECT-like");

    let rows: Vec<MySqlRow> = sqlx::query(sql)
        .fetch_all(&mut **conn)
        .await
        .map_err(map_sqlx_error)?;

    // 列名 / 列类型取第一行的（空结果集时尝试用 query.describe 但代价高，先简化处理）
    // 类型用 sqlx::TypeInfo::name()，MySQL 返回大写如 "BIGINT" / "VARCHAR" / "DATETIME"
    let (columns, column_types): (Vec<String>, Vec<String>) = if let Some(first) = rows.first() {
        first
            .columns()
            .iter()
            .map(|c| (c.name().to_string(), c.type_info().name().to_string()))
            .unzip()
    } else {
        (Vec::new(), Vec::new())
    };

    let domain_rows: Vec<Row> = rows
        .iter()
        .map(|r| Row {
            values: decode_row(r),
        })
        .collect();

    Ok(QueryResult {
        columns,
        column_types,
        rows: domain_rows,
        affected_rows: 0,
        elapsed_ms: 0,        // 外层填
        warnings: Vec::new(), // 外层在每条 statement 后通过 fetch_warnings 注入
    })
}

async fn execute_dml(conn: &mut PoolConnection<MySql>, sql: &str) -> Result<QueryResult> {
    debug!(?sql, "executing DML/DDL");

    let result = conn.execute(sql).await.map_err(map_sqlx_error)?;

    Ok(QueryResult {
        columns: Vec::new(),
        column_types: Vec::new(),
        rows: Vec::new(),
        affected_rows: result.rows_affected(),
        elapsed_ms: 0,        // 外层填
        warnings: Vec::new(), // 外层在每条 statement 后通过 fetch_warnings 注入
    })
}

/// 取当前会话的 SHOW WARNINGS 列表（对刚执行的语句有效；下一条语句会清空）
///
/// 返回空 Vec 表示无警告或抓取失败（失败仅 warn 不影响主流程）
/// MySQL 协议层 SHOW WARNINGS 列固定为 (Level VARCHAR, Code INT, Message TEXT)
///
/// **必须用 `sqlx::raw_sql`（text protocol）**：sqlx 默认 query/query_as 走 prepared statement，
/// 但 MySQL 的 SHOW 系列命令不支持 PREPARE 协议，会报 1295（HY000）。
/// raw_sql 直接发文本到服务端，绕开 PREPARE 阶段
async fn fetch_warnings(conn: &mut PoolConnection<MySql>) -> Vec<Warning> {
    use sqlx::Row as _;
    // 走 prepared statement 路径（sqlx::query 默认行为）
    //
    // 历史选择：理想做法是 sqlx::raw_sql（text protocol）避开 1295 限制，但 sqlx 0.8
    // 的 raw_sql + async_trait + tokio::spawn 组合下 HRTB 推断失败（已尝试 Box::pin /
    // ownership transfer / .boxed() 都不行），强行做需要 unsafe transmute 违反生产原则
    //
    // 折衷：用 query 走 prepared，捕获 1295 静默。MySQL 8.0.14+ prepared 支持 SHOW WARNINGS
    // 该路径正常工作；老版本（含部分 8.0.x 早期）静默不报警避免日志噪音
    let rows: std::result::Result<Vec<sqlx::mysql::MySqlRow>, sqlx::Error> =
        sqlx::query("SHOW WARNINGS").fetch_all(&mut **conn).await;
    match rows {
        Ok(rows) => rows
            .into_iter()
            .filter_map(|row| {
                // SHOW WARNINGS 的列固定顺序：Level / Code / Message
                let level: String = row.try_get(0).ok()?;
                let code: u32 = row.try_get(1).ok()?;
                let message: String = row.try_get(2).ok()?;
                Some(Warning {
                    level,
                    code,
                    message,
                })
            })
            .collect(),
        Err(e) => {
            // 1295 = "This command is not supported in the prepared statement protocol"
            // 静默：服务器版本限制，不影响主流程
            let is_unsupported =
                e.as_database_error().and_then(|d| d.code()).as_deref() == Some("1295");
            if !is_unsupported {
                warn!(error = %e, "fetch SHOW WARNINGS failed (non-fatal)");
            }
            Vec::new()
        }
    }
}

/// 发 `KILL QUERY <thread_id>` 中断目标 session 的当前语句（不断 connection）
///
/// 必须用单独的连接发，不能复用执行查询的那条 conn（它正在等 await）
pub async fn kill_query(pool: &MySqlPool, thread_id: u64) -> Result<()> {
    info!(thread_id, "sending KILL QUERY");
    // KILL QUERY 后端语法支持参数化，但部分 mysql 版本不严格校验，
    // 直接 format 到 SQL 里更稳；thread_id 是 u64，不存在注入风险
    let sql = format!("KILL QUERY {thread_id}");
    pool.execute(sql.as_str()).await.map_err(map_sqlx_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_select() {
        assert!(is_query_returning_rows("SELECT 1"));
        assert!(is_query_returning_rows("  select * from t"));
        assert!(is_query_returning_rows("SHOW TABLES"));
        assert!(is_query_returning_rows("DESC users"));
        assert!(is_query_returning_rows("EXPLAIN SELECT 1"));
        assert!(is_query_returning_rows(
            "WITH t AS (SELECT 1) SELECT * FROM t"
        ));
    }

    #[test]
    fn split_single() {
        assert_eq!(split_statements("SELECT 1"), vec!["SELECT 1".to_string()]);
        assert_eq!(split_statements("SELECT 1;"), vec!["SELECT 1".to_string()]);
    }

    #[test]
    fn split_multi() {
        let s = split_statements("SELECT 1; SELECT 2;\n SELECT 3");
        assert_eq!(
            s,
            vec![
                "SELECT 1".to_string(),
                "SELECT 2".to_string(),
                "SELECT 3".to_string(),
            ]
        );
    }

    #[test]
    fn split_skips_semicolon_in_string() {
        let s = split_statements("SELECT 'a;b'; SELECT 2");
        assert_eq!(s, vec!["SELECT 'a;b'".to_string(), "SELECT 2".to_string(),]);
    }

    #[test]
    fn split_skips_semicolon_in_comment() {
        let s = split_statements("SELECT 1 -- a;b\n; SELECT 2");
        assert_eq!(s.len(), 2);
        assert!(s[0].contains("SELECT 1"));
        assert_eq!(s[1], "SELECT 2");
    }

    #[test]
    fn split_skips_empty_statements() {
        // 连续 `;;` 或末尾换行不产生空语句
        let s = split_statements(";; SELECT 1 ;;\n;");
        assert_eq!(s, vec!["SELECT 1".to_string()]);
    }

    #[test]
    fn detect_dml() {
        assert!(!is_query_returning_rows("INSERT INTO t VALUES (1)"));
        assert!(!is_query_returning_rows("UPDATE t SET a=1"));
        assert!(!is_query_returning_rows("DELETE FROM t"));
        assert!(!is_query_returning_rows("CREATE TABLE t (id INT)"));
        assert!(!is_query_returning_rows("ALTER TABLE t ADD COLUMN x INT"));
        assert!(!is_query_returning_rows("DROP TABLE t"));
    }

    // === inject_limit_if_needed ===

    #[test]
    fn inject_basic_select() {
        let r = inject_limit_if_needed("SELECT * FROM t", Some(500));
        assert_eq!(r.as_deref(), Some("SELECT * FROM t LIMIT 500"));
    }

    #[test]
    fn inject_keeps_trailing_semicolon() {
        let r = inject_limit_if_needed("SELECT 1 ;", Some(100));
        assert_eq!(r.as_deref(), Some("SELECT 1 LIMIT 100;"));
    }

    #[test]
    fn inject_skip_when_already_has_limit() {
        assert!(inject_limit_if_needed("SELECT * FROM t LIMIT 10", Some(500)).is_none());
        assert!(inject_limit_if_needed("SELECT * FROM t LIMIT 10, 20", Some(500)).is_none());
        assert!(inject_limit_if_needed("SELECT * FROM t LIMIT 10 OFFSET 5", Some(500)).is_none());
    }

    #[test]
    fn inject_skip_when_disabled() {
        // None 或 0 都不注入
        assert!(inject_limit_if_needed("SELECT 1", None).is_none());
        assert!(inject_limit_if_needed("SELECT 1", Some(0)).is_none());
    }

    #[test]
    fn inject_skip_non_select() {
        assert!(inject_limit_if_needed("SHOW TABLES", Some(500)).is_none());
        assert!(inject_limit_if_needed("DESC users", Some(500)).is_none());
        assert!(inject_limit_if_needed("EXPLAIN SELECT 1", Some(500)).is_none());
        assert!(inject_limit_if_needed("INSERT INTO t VALUES (1)", Some(500)).is_none());
    }

    #[test]
    fn inject_works_for_with_cte() {
        let r = inject_limit_if_needed("WITH t AS (SELECT 1) SELECT * FROM t", Some(500));
        assert_eq!(
            r.as_deref(),
            Some("WITH t AS (SELECT 1) SELECT * FROM t LIMIT 500")
        );
    }

    #[test]
    fn inject_skips_when_inner_subquery_has_limit_only() {
        // 子查询 LIMIT 不影响外层；最外层无 LIMIT 仍要注入
        // 末尾 64 字符扫描看到 "LIMIT 5)" — 会被识别为已含 LIMIT
        // 这是"宁可漏注入"的代价；在显式分页 SQL 里几乎不会发生
        let r = inject_limit_if_needed("SELECT * FROM (SELECT * FROM t LIMIT 5) x", Some(500));
        assert!(r.is_none());
    }

    #[test]
    fn inject_lowercase_select_works() {
        let r = inject_limit_if_needed("select 1", Some(50));
        assert_eq!(r.as_deref(), Some("select 1 LIMIT 50"));
    }

    #[test]
    fn contains_word_avoids_substring_match() {
        // column 名叫 limit 不应被识别为 LIMIT 子句
        assert!(!contains_word("SELECT MY_LIMITS FROM T", "LIMIT"));
        assert!(contains_word("SELECT * FROM T LIMIT 10", "LIMIT"));
    }

    // === sql_has_no_limit_marker ===

    #[test]
    fn marker_detected() {
        assert!(sql_has_no_limit_marker("-- ramag:no-limit\nSELECT 1"));
        assert!(sql_has_no_limit_marker("SELECT 1\n-- ramag:no-limit\n"));
        // 大小写不敏感
        assert!(sql_has_no_limit_marker("-- RAMAG:NO-LIMIT\nSELECT 1"));
        // 前面有空格也行
        assert!(sql_has_no_limit_marker("   --   ramag:no-limit\n"));
    }

    #[test]
    fn marker_not_detected_in_string() {
        // 字符串里出现的 "ramag:no-limit" 不算（因为不是行注释开头）
        assert!(!sql_has_no_limit_marker("SELECT 'ramag:no-limit'"));
    }
}
