//! 结果集分页：仅"未手写 LIMIT/OFFSET 的单条 SELECT/WITH"启用，翻页时重写 LIMIT/OFFSET 重跑

use ramag_domain::entities::DriverKind;

use super::sql_utils::{has_top_level_keyword, split_sql_statements};

/// 当前 Tab 的分页状态。仅当本次 run 命中分页资格时为 Some
pub(crate) struct Pager {
    /// 去掉尾分号、未注入 LIMIT 的原始单条语句，翻页时以它为底重写
    pub(crate) base_sql: String,
    /// 0-based 页码
    pub(crate) page: usize,
    /// 本页行数打满页大小即认为可能还有下一页（不跑 COUNT，避免大表代价）
    pub(crate) has_more: bool,
}

/// 分页资格判定：单条裸 SELECT/WITH（无顶层 LIMIT/OFFSET）才返回语句体。
/// 多语句 / DML / 用户已写 LIMIT / 注释开头（含 no-limit 标记场景）一律 None
pub(super) fn paging_base_sql(sql: &str, driver: DriverKind) -> Option<String> {
    let stmts = split_sql_statements(sql, driver);
    let [stmt] = stmts.as_slice() else {
        return None;
    };
    let trimmed = stmt.trim();
    let head: String = trimmed
        .chars()
        .take(8)
        .collect::<String>()
        .to_ascii_uppercase();
    if !(head.starts_with("SELECT") || head.starts_with("WITH")) {
        return None;
    }
    let upper = trimmed.to_ascii_uppercase();
    if has_top_level_keyword(&upper, "LIMIT") || has_top_level_keyword(&upper, "OFFSET") {
        return None;
    }
    Some(trimmed.trim_end_matches(';').trim_end().to_string())
}

/// 按页码生成翻页 SQL：第 0 页只注入 LIMIT（与自动注入行为一致），其后追加 OFFSET
pub(super) fn page_sql(base: &str, page_size: usize, page: usize) -> String {
    if page == 0 {
        format!("{base} LIMIT {page_size}")
    } else {
        format!("{base} LIMIT {page_size} OFFSET {}", page * page_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligible_plain_select() {
        let base = paging_base_sql("SELECT * FROM t;", DriverKind::Mysql);
        assert_eq!(base.as_deref(), Some("SELECT * FROM t"));
    }

    #[test]
    fn eligible_with_cte() {
        let base = paging_base_sql("WITH x AS (SELECT 1) SELECT * FROM x", DriverKind::Postgres);
        assert!(base.is_some());
    }

    #[test]
    fn rejects_user_limit_or_offset() {
        assert!(paging_base_sql("SELECT * FROM t LIMIT 10", DriverKind::Mysql).is_none());
        assert!(paging_base_sql("SELECT * FROM t OFFSET 5", DriverKind::Postgres).is_none());
    }

    #[test]
    fn keeps_subquery_limit_eligible() {
        let sql = "SELECT * FROM (SELECT * FROM t LIMIT 10) x";
        assert!(paging_base_sql(sql, DriverKind::Mysql).is_some());
    }

    #[test]
    fn rejects_multi_statement_and_dml() {
        assert!(paging_base_sql("SELECT 1; SELECT 2", DriverKind::Mysql).is_none());
        assert!(paging_base_sql("UPDATE t SET a=1", DriverKind::Mysql).is_none());
        assert!(paging_base_sql("SHOW TABLES", DriverKind::Mysql).is_none());
    }

    #[test]
    fn rejects_comment_leading_no_limit_marker() {
        // 注释开头不以 SELECT 起始，自动跳过（与 LIMIT 注入的判定一致）
        assert!(paging_base_sql("-- ramag:no-limit\nSELECT * FROM t", DriverKind::Mysql).is_none());
    }

    #[test]
    fn page_sql_first_and_next() {
        assert_eq!(
            page_sql("SELECT * FROM t", 100, 0),
            "SELECT * FROM t LIMIT 100"
        );
        assert_eq!(
            page_sql("SELECT * FROM t", 100, 2),
            "SELECT * FROM t LIMIT 100 OFFSET 200"
        );
    }
}
