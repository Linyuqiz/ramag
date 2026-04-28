//! SQL 自动补全
//!
//! 实现 gpui-component 的 `CompletionProvider` trait：
//! - Phase 1：静态 SQL 关键字
//! - Phase 2：当前连接默认 schema 的表名（`FROM` / `JOIN` 等位置后）
//! - Phase 3：当前查询涉及表的列名（`SELECT` / `WHERE` 等位置后；待实现）

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Result;
use gpui::{Context, Task, Window};
use gpui_component::RopeExt;
use gpui_component::input::{CompletionProvider, InputState};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    InsertReplaceEdit,
};
use parking_lot::RwLock;
use ropey::Rope;

/// MySQL 常见关键字 + 函数
const SQL_KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "GROUP BY",
    "ORDER BY",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "INSERT INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE FROM",
    "JOIN",
    "LEFT JOIN",
    "RIGHT JOIN",
    "INNER JOIN",
    "OUTER JOIN",
    "FULL JOIN",
    "CROSS JOIN",
    "ON",
    "USING",
    "UNION",
    "UNION ALL",
    "INTERSECT",
    "EXCEPT",
    "DISTINCT",
    "ALL",
    "AND",
    "OR",
    "NOT",
    "IN",
    "BETWEEN",
    "LIKE",
    "IS NULL",
    "IS NOT NULL",
    "EXISTS",
    "ANY",
    "SOME",
    "ASC",
    "DESC",
    "CREATE TABLE",
    "DROP TABLE",
    "ALTER TABLE",
    "TRUNCATE TABLE",
    "CREATE INDEX",
    "DROP INDEX",
    "CREATE VIEW",
    "DROP VIEW",
    "AS",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "IF",
    "IFNULL",
    "COALESCE",
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "GROUP_CONCAT",
    "NOW",
    "CURRENT_TIMESTAMP",
    "DATE",
    "DATE_FORMAT",
    "DATEDIFF",
    "CONCAT",
    "SUBSTRING",
    "LENGTH",
    "TRIM",
    "UPPER",
    "LOWER",
    "CAST",
    "CONVERT",
    "TRUE",
    "FALSE",
    "NULL",
];

/// MySQL 系统内置库（默认隐藏；下拉/表树排序时下沉）
pub const SYSTEM_SCHEMAS: &[&str] = &["mysql", "information_schema", "performance_schema", "sys"];

/// 大小写无关地判断库名是否系统内置
pub fn is_system_schema(name: &str) -> bool {
    SYSTEM_SCHEMAS.iter().any(|s| s.eq_ignore_ascii_case(name))
}

/// 当前连接的 schema 缓存（共享给补全 provider + DB 下拉 + 表树）
///
/// 由 ConnectionSession 在连接建立后异步填充，QueryTab 创建编辑器时
/// 把这个 Arc 传给 SqlCompletionProvider；后续即使重建编辑器也读同一份。
#[derive(Default)]
pub struct SchemaCache {
    /// schema → 表名列表
    pub tables: HashMap<String, Vec<String>>,
    /// (schema, table) → 列名列表（Phase 3 用，当前未填）
    pub columns: HashMap<(String, String), Vec<String>>,
    /// 默认 schema（连接配置里的 database 字段）
    pub default_schema: Option<String>,
    /// 当前连接已知的所有 schema 名（不论是否展开）
    /// 由 TableTreePanel 在 list_schemas 成功后写入；DB 下拉读取
    pub all_schemas: Vec<String>,
    /// 表树侧"显示系统库"toggle 的当前状态
    /// 默认 false（隐藏）；DB 下拉读取此值决定是否展示系统库
    pub show_system: bool,
}

impl SchemaCache {
    pub fn new_shared() -> Arc<RwLock<Self>> {
        Arc::new(RwLock::new(Self::default()))
    }

    /// 取所有可补全的表名（默认 schema 优先，其余次之）
    pub fn all_tables(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(d) = &self.default_schema {
            if let Some(ts) = self.tables.get(d) {
                out.extend(ts.iter().cloned());
            }
        }
        for (s, ts) in &self.tables {
            if Some(s) != self.default_schema.as_ref() {
                out.extend(ts.iter().cloned());
            }
        }
        out
    }
}

/// SQL 上下文：根据光标前的最后一个关键字猜测应补全什么
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SqlContext {
    /// 应补表名：FROM / JOIN / INTO / UPDATE / TABLE 后
    Table,
    /// 应补列名：SELECT 后（FROM 之前）/ WHERE / AND / OR / ON / HAVING / SET /
    /// ORDER BY / GROUP BY 后
    Column,
    /// 其他位置：仅补关键字
    Other,
}

/// 通过 cursor 前的纯大写文本，找最近的关键字判定上下文
fn detect_context(before_cursor_upper: &str) -> SqlContext {
    let tokens: Vec<String> = before_cursor_upper
        .split_ascii_whitespace()
        .map(|t| {
            t.trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .to_string()
        })
        .filter(|t| !t.is_empty())
        .collect();

    // 倒着扫，碰到第一个能定上下文的 token 就返回
    for (i, t) in tokens.iter().enumerate().rev() {
        let t = t.as_str();

        // 多词关键字：BY 前面是 ORDER / GROUP → 列名上下文
        if t == "BY" && i > 0 {
            let prev = tokens[i - 1].as_str();
            if prev == "ORDER" || prev == "GROUP" {
                return SqlContext::Column;
            }
        }

        match t {
            // 表名上下文
            "FROM" | "JOIN" | "INTO" | "UPDATE" | "TABLE" => return SqlContext::Table,
            // 列名上下文
            "SELECT" | "WHERE" | "AND" | "OR" | "ON" | "USING" | "HAVING" | "SET" | "DISTINCT" => {
                return SqlContext::Column;
            }
            _ => {}
        }
    }
    SqlContext::Other
}

/// 公开版本：让 QueryTab 编辑器变化时可以预拉这些表的列结构
/// 返回 (schema_可选, table) 对，schema 来自 `db.table` 这种全限定形式
pub fn extract_tables_in_use_for_prefetch(sql: &str) -> Vec<(Option<String>, String)> {
    extract_tables_with_schema(sql)
}

/// 从 SQL 中提取 FROM / JOIN / UPDATE / INTO 后的表名（仅名字版本）
/// 用于列名补全的查表名匹配（跨 schema）
fn extract_tables_in_use(sql: &str) -> Vec<String> {
    extract_tables_with_schema(sql)
        .into_iter()
        .map(|(_, t)| t)
        .collect()
}

/// 提取 (schema, table) 对：schema 来自全限定 `schema.table` 形式
/// 若是裸表名（无 schema 前缀），返回 (None, table)
fn extract_tables_with_schema(sql: &str) -> Vec<(Option<String>, String)> {
    let upper: Vec<String> = sql
        .split_ascii_whitespace()
        .map(|t| t.to_ascii_uppercase())
        .collect();
    let orig: Vec<&str> = sql.split_ascii_whitespace().collect();

    let mut tables = Vec::new();
    for i in 0..upper.len() {
        let kw = upper[i].trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_');
        if matches!(kw, "FROM" | "JOIN" | "INTO" | "UPDATE") && i + 1 < orig.len() {
            let raw = orig[i + 1];
            // 去反引号 / 引号 / 括号等，仅保留 [\w.]
            let cleaned: String = raw
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
                .collect();
            // 按 . 拆：[schema, table] 或 [table]
            let parts: Vec<&str> = cleaned.split('.').filter(|s| !s.is_empty()).collect();
            match parts.as_slice() {
                [t] => tables.push((None, (*t).to_string())),
                [s, t] => tables.push((Some((*s).to_string()), (*t).to_string())),
                [_, s, t] => {
                    // catalog.schema.table 形式：取后两段
                    tables.push((Some((*s).to_string()), (*t).to_string()))
                }
                _ => {}
            }
        }
    }
    tables
}

/// 构造一个 CompletionItem 的小帮手，统一格式
fn make_item(
    label: String,
    kind: CompletionItemKind,
    detail: Option<&str>,
    range: lsp_types::Range,
) -> CompletionItem {
    CompletionItem {
        label: label.clone(),
        kind: Some(kind),
        detail: detail.map(|s| s.to_string()),
        text_edit: Some(CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
            new_text: label,
            insert: range,
            replace: range,
        })),
        ..Default::default()
    }
}

/// SQL 补全 provider：关键字 + 表名（基于 cache）
pub struct SqlCompletionProvider {
    cache: Arc<RwLock<SchemaCache>>,
}

impl SqlCompletionProvider {
    pub fn new_rc(cache: Arc<RwLock<SchemaCache>>) -> Rc<dyn CompletionProvider> {
        Rc::new(Self { cache })
    }
}

impl CompletionProvider for SqlCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        _cx: &mut Context<InputState>,
    ) -> Task<Result<CompletionResponse>> {
        let text = rope.to_string();
        let bytes = text.as_bytes();
        let real_offset = offset.min(bytes.len());

        // 取光标前的"单词"作为补全前缀
        let mut start = real_offset;
        while start > 0 {
            let b = bytes[start - 1];
            let is_word = b.is_ascii_alphanumeric() || b == b'_';
            if !is_word {
                break;
            }
            start -= 1;
        }
        let prefix = &text[start..real_offset];
        if prefix.is_empty() {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        }

        let start_pos = rope.offset_to_position(start);
        let end_pos = rope.offset_to_position(real_offset);
        let replace_range = lsp_types::Range::new(start_pos, end_pos);

        let prefix_upper = prefix.to_ascii_uppercase();
        let prefix_lower = prefix.to_ascii_lowercase();

        // 上下文判定：取前缀单词之前的全部文本（不含当前正在敲的）
        let before = &text[..start];
        let context = detect_context(&before.to_ascii_uppercase());

        let mut items: Vec<CompletionItem> = Vec::new();

        match context {
            // 1. Table 上下文：建议表名（默认 schema 优先）
            SqlContext::Table => {
                let cache = self.cache.read();
                for name in cache.all_tables() {
                    if name.to_ascii_lowercase().starts_with(&prefix_lower) {
                        items.push(make_item(
                            name.clone(),
                            CompletionItemKind::CLASS,
                            Some("table"),
                            replace_range,
                        ));
                        if items.len() >= 30 {
                            break;
                        }
                    }
                }
            }
            // 2. Column 上下文：解析 FROM 找出涉及的表，从 cache.columns 取列
            // 注意：用整段 SQL 解析（不只是光标前），因为 FROM 可能在光标后
            // 如 `SELECT t|<cursor> FROM users`
            SqlContext::Column => {
                let tables_in_use = extract_tables_in_use(&text);
                let cache = self.cache.read();
                let mut seen = std::collections::HashSet::new();
                for table_name in &tables_in_use {
                    for ((_schema, t), cols) in cache.columns.iter() {
                        if !t.eq_ignore_ascii_case(table_name) {
                            continue;
                        }
                        for col in cols {
                            if !seen.insert(col.clone()) {
                                continue;
                            }
                            if col.to_ascii_lowercase().starts_with(&prefix_lower) {
                                items.push(make_item(
                                    col.clone(),
                                    CompletionItemKind::FIELD,
                                    Some("column"),
                                    replace_range,
                                ));
                                if items.len() >= 30 {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            SqlContext::Other => {}
        }

        // 3. 关键字（任何上下文都可以补，作为兜底；总数控制在 50 内）
        for kw in SQL_KEYWORDS {
            if items.len() >= 50 {
                break;
            }
            if kw.starts_with(&prefix_upper) {
                items.push(make_item(
                    kw.to_string(),
                    CompletionItemKind::KEYWORD,
                    None,
                    replace_range,
                ));
            }
        }

        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(
        &self,
        _offset: usize,
        new_text: &str,
        _cx: &mut Context<InputState>,
    ) -> bool {
        new_text.chars().all(|c| c.is_alphanumeric() || c == '_')
    }
}

/// 列过滤框补全 provider：候选列 = 当前结果集列名（不属于 SchemaCache）
///
/// 由 ResultPanel 创建并把 `Arc<RwLock<Vec<String>>>` 共享：每次新查询返回时
/// ResultPanel 把结果列名写入这个 Arc，下次用户在过滤框敲字就能看到最新列。
///
/// Token 切分：以光标前最近的 `,` 为左边界，跳过前导空格
/// 目的：用户在 `id, na` 状态下敲字，只匹配 `na` 这段而非整个文本
pub struct ColumnFilterCompletionProvider {
    columns: Arc<RwLock<Vec<String>>>,
}

impl ColumnFilterCompletionProvider {
    pub fn new_rc(columns: Arc<RwLock<Vec<String>>>) -> Rc<dyn CompletionProvider> {
        Rc::new(Self { columns })
    }
}

impl CompletionProvider for ColumnFilterCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        _cx: &mut Context<InputState>,
    ) -> Task<Result<CompletionResponse>> {
        let text = rope.to_string();
        let bytes = text.as_bytes();
        let real_offset = offset.min(bytes.len());

        // 找当前 token 起点：从光标向前扫到最近的逗号（或文本起点）
        let mut tok_start = real_offset;
        while tok_start > 0 && bytes[tok_start - 1] != b',' {
            tok_start -= 1;
        }
        // 跳过前导空格
        while tok_start < real_offset && bytes[tok_start] == b' ' {
            tok_start += 1;
        }
        let prefix = &text[tok_start..real_offset];
        if prefix.is_empty() {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        }
        let prefix_lower = prefix.to_ascii_lowercase();

        let start_pos = rope.offset_to_position(tok_start);
        let end_pos = rope.offset_to_position(real_offset);
        let replace_range = lsp_types::Range::new(start_pos, end_pos);

        // 已经填进过滤框的列（其它 token）不再建议，避免重复
        let already: std::collections::HashSet<String> = text
            .split(',')
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty() && *s != prefix_lower)
            .collect();

        let cols = self.columns.read();
        let mut items: Vec<CompletionItem> = Vec::new();
        for name in cols.iter() {
            let lc = name.to_ascii_lowercase();
            // 子串匹配（与表格过滤逻辑一致：大小写不敏感 contains）
            if !lc.contains(&prefix_lower) {
                continue;
            }
            if already.contains(&lc) {
                continue;
            }
            items.push(make_item(
                name.clone(),
                CompletionItemKind::FIELD,
                Some("column"),
                replace_range,
            ));
            if items.len() >= 50 {
                break;
            }
        }
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(
        &self,
        _offset: usize,
        new_text: &str,
        _cx: &mut Context<InputState>,
    ) -> bool {
        // 字母 / 数字 / 下划线触发；逗号不触发（逗号后用户还要输入下一个 token）
        new_text.chars().all(|c| c.is_alphanumeric() || c == '_')
    }
}

#[cfg(test)]
mod tests {
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
        let mut c = SchemaCache::default();
        c.default_schema = Some("midas".to_string());
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
}
