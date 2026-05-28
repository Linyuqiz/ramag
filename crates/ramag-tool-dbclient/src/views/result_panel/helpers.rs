//! ResultPanel 自由函数：主键定位 / WHERE 拼装 / 类型转换 / DML LIMIT 方言 / 表名提取 / 输入校验

use gpui::Entity;
use gpui_component::input::InputState;
use ramag_domain::entities::{Column, ColumnKind, QueryResult, Value};

/// 新增草稿行。表名在 INSERT 时由 `extract_first_table_ref` 从 SQL 反推，与 UPDATE/DELETE 一致
pub(crate) struct PendingInsert {
    pub columns: Vec<Column>,
    pub inputs: Vec<Entity<InputState>>,
}

/// 用户输入 → Value。Ok(Some)=有值、Ok(None)=留空且有 default 走 DB DEFAULT、Err=非法
pub(super) fn parse_value_for_kind(
    kind: ColumnKind,
    text: &str,
    nullable: bool,
    has_default: bool,
) -> Result<Option<Value>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        if nullable {
            return Ok(Some(Value::Null));
        }
        if has_default {
            return Ok(None);
        }
        return Err("必填".to_string());
    }
    if trimmed.eq_ignore_ascii_case("NULL") {
        if nullable {
            return Ok(Some(Value::Null));
        }
        return Err("不可为 NULL".to_string());
    }
    match kind {
        ColumnKind::Integer => trimmed
            .parse::<i64>()
            .map(|i| Some(Value::Int(i)))
            .map_err(|_| format!("不是合法整数: {trimmed}")),
        ColumnKind::Decimal | ColumnKind::Float => trimmed
            .parse::<f64>()
            .map(|f| Some(Value::Float(f)))
            .map_err(|_| format!("不是合法数值: {trimmed}")),
        ColumnKind::Bool => match trimmed {
            "1" | "true" | "TRUE" | "True" => Ok(Some(Value::Bool(true))),
            "0" | "false" | "FALSE" | "False" => Ok(Some(Value::Bool(false))),
            _ => Err(format!("布尔值需 1/0/true/false: {trimmed}")),
        },
        _ => Ok(Some(Value::Text(trimmed.to_string()))),
    }
}

/// 推断主键列：优先名为 `id`，其次任意 `_id` 后缀列；都没有返回 None
pub(super) fn find_pk_idx(result: &QueryResult) -> Option<usize> {
    result
        .columns
        .iter()
        .position(|c| c.eq_ignore_ascii_case("id"))
        .or_else(|| {
            result
                .columns
                .iter()
                .position(|c| c.to_ascii_lowercase().ends_with("_id"))
        })
}

/// 构造按主键的 WHERE 子句：主键缺失时回退所有列等值（脆弱但安全）
///
/// `driver` 决定标识符引号字符（MySQL 反引号 / PG 双引号）
pub(super) fn build_pk_where(
    result: &QueryResult,
    row: &ramag_domain::entities::Row,
    driver: ramag_domain::entities::DriverKind,
) -> String {
    if let Some(idx) = find_pk_idx(result) {
        let col = result.columns.get(idx).cloned().unwrap_or_default();
        let val = row
            .values
            .get(idx)
            .map(|v| v.to_sql_literal())
            .unwrap_or_else(|| "NULL".into());
        format!("{} = {}", driver.quote_identifier(&col), val)
    } else {
        result
            .columns
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let v = row
                    .values
                    .get(i)
                    .map(|v| v.to_sql_literal())
                    .unwrap_or_else(|| "NULL".into());
                format!("{} = {}", driver.quote_identifier(c), v)
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    }
}

/// 按原 cell 类型把用户输入转换成新的 Value（同时供本地刷新 + SQL 字面量）
fn build_new_value_for_old(old: &Value, new_text: &str) -> Value {
    match old {
        Value::Null => {
            if new_text.is_empty() || new_text.eq_ignore_ascii_case("NULL") {
                Value::Null
            } else {
                Value::Text(new_text.to_string())
            }
        }
        Value::Int(_) => new_text
            .parse::<i64>()
            .map(Value::Int)
            .unwrap_or_else(|_| Value::Text(new_text.to_string())),
        Value::Float(_) => new_text
            .parse::<f64>()
            .map(Value::Float)
            .unwrap_or_else(|_| Value::Text(new_text.to_string())),
        Value::Bool(_) => match new_text.trim() {
            "1" | "true" | "TRUE" | "True" => Value::Bool(true),
            "0" | "false" | "FALSE" | "False" => Value::Bool(false),
            _ => Value::Text(new_text.to_string()),
        },
        _ => Value::Text(new_text.to_string()),
    }
}

/// 公开版本：用于 ops::apply_cell_update_async 同步本地 cell
pub(super) fn build_new_value(old: &Value, new_text: &str) -> Value {
    build_new_value_for_old(old, new_text)
}

/// SQL 字面量包装（apply_cell_update_async 用）：build → to_sql_literal
pub(super) fn escape_new_value_for_old(old: &Value, new_text: &str) -> String {
    build_new_value_for_old(old, new_text).to_sql_literal()
}

/// 单行 DML LIMIT 子句。MySQL ` LIMIT 1` 防误删；PG / Redis / MongoDB 不支持，返回空
pub(super) fn dml_row_limit(driver: ramag_domain::entities::DriverKind) -> &'static str {
    match driver {
        ramag_domain::entities::DriverKind::Mysql => " LIMIT 1",
        ramag_domain::entities::DriverKind::Postgres
        | ramag_domain::entities::DriverKind::Redis
        | ramag_domain::entities::DriverKind::Mongodb => "",
    }
}

/// 从 SQL 提取第一个表引用（按 driver 方言加引号），用于复制 INSERT 时的目标表
pub(super) fn extract_first_table_ref(
    sql: &str,
    driver: ramag_domain::entities::DriverKind,
) -> Option<String> {
    let tables = crate::sql_completion::extract_tables_in_use_for_prefetch(sql);
    let (maybe_schema, table) = tables.into_iter().next()?;
    let table_q = driver.quote_identifier(&table);
    Some(match maybe_schema {
        Some(s) => format!("{}.{}", driver.quote_identifier(&s), table_q),
        None => table_q,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ramag_domain::entities::{ColumnKind, DriverKind, QueryResult, Row, Value};

    /// 通过 `to_sql_literal` 把 Value 拍成可比较字符串（Value 没实现 PartialEq）
    fn lit(v: &Value) -> String {
        v.to_sql_literal()
    }

    fn make_result(cols: &[&str]) -> QueryResult {
        QueryResult {
            columns: cols.iter().map(|s| s.to_string()).collect(),
            column_types: vec![String::new(); cols.len()],
            rows: vec![],
            warnings: vec![],
            elapsed_ms: 0,
            affected_rows: 0,
        }
    }

    #[test]
    fn parse_value_empty_nullable() {
        let v = parse_value_for_kind(ColumnKind::Text, "", true, false).unwrap();
        assert_eq!(lit(v.as_ref().unwrap()), "NULL");
    }

    #[test]
    fn parse_value_empty_with_default() {
        let v = parse_value_for_kind(ColumnKind::Text, "  ", false, true).unwrap();
        assert!(v.is_none(), "留空 + 有 default → 跳过让 DB 用 DEFAULT");
    }

    #[test]
    fn parse_value_empty_required() {
        let err = parse_value_for_kind(ColumnKind::Text, "", false, false).unwrap_err();
        assert!(err.contains("必填"));
    }

    #[test]
    fn parse_value_explicit_null_nullable() {
        for s in ["NULL", "null", "Null"] {
            let v = parse_value_for_kind(ColumnKind::Integer, s, true, false).unwrap();
            assert_eq!(lit(v.as_ref().unwrap()), "NULL", "input={s}");
        }
    }

    #[test]
    fn parse_value_explicit_null_not_nullable() {
        let err = parse_value_for_kind(ColumnKind::Integer, "NULL", false, true).unwrap_err();
        assert!(err.contains("不可为 NULL"));
    }

    #[test]
    fn parse_value_integer_ok() {
        let v = parse_value_for_kind(ColumnKind::Integer, "42", false, false).unwrap();
        assert_eq!(lit(v.as_ref().unwrap()), "42");
    }

    #[test]
    fn parse_value_integer_negative() {
        let v = parse_value_for_kind(ColumnKind::Integer, "-7", false, false).unwrap();
        assert_eq!(lit(v.as_ref().unwrap()), "-7");
    }

    #[test]
    fn parse_value_integer_invalid() {
        let err = parse_value_for_kind(ColumnKind::Integer, "abc", false, false).unwrap_err();
        assert!(err.contains("不是合法整数"));
    }

    #[test]
    fn parse_value_float_ok() {
        let v = parse_value_for_kind(ColumnKind::Float, "3.5", false, false).unwrap();
        assert!(matches!(v, Some(Value::Float(_))));
        assert_eq!(lit(v.as_ref().unwrap()), "3.5");
    }

    #[test]
    fn parse_value_decimal_ok() {
        let v = parse_value_for_kind(ColumnKind::Decimal, "1.5", false, false).unwrap();
        assert!(matches!(v, Some(Value::Float(_))));
        assert_eq!(lit(v.as_ref().unwrap()), "1.5");
    }

    #[test]
    fn parse_value_bool_truthy() {
        for s in ["1", "true", "TRUE", "True"] {
            let v = parse_value_for_kind(ColumnKind::Bool, s, false, false).unwrap();
            assert_eq!(lit(v.as_ref().unwrap()), "TRUE", "input={s}");
        }
    }

    #[test]
    fn parse_value_bool_falsy() {
        for s in ["0", "false", "FALSE", "False"] {
            let v = parse_value_for_kind(ColumnKind::Bool, s, false, false).unwrap();
            assert_eq!(lit(v.as_ref().unwrap()), "FALSE", "input={s}");
        }
    }

    #[test]
    fn parse_value_bool_invalid() {
        let err = parse_value_for_kind(ColumnKind::Bool, "yes", false, false).unwrap_err();
        assert!(err.contains("布尔值"));
    }

    #[test]
    fn parse_value_text_trimmed() {
        let v = parse_value_for_kind(ColumnKind::Text, "  hello  ", false, false).unwrap();
        assert_eq!(lit(v.as_ref().unwrap()), "'hello'");
    }

    #[test]
    fn find_pk_idx_prefers_id() {
        let r = make_result(&["name", "id", "user_id"]);
        assert_eq!(find_pk_idx(&r), Some(1));
    }

    #[test]
    fn find_pk_idx_case_insensitive() {
        let r = make_result(&["name", "ID"]);
        assert_eq!(find_pk_idx(&r), Some(1));
    }

    #[test]
    fn find_pk_idx_falls_back_to_id_suffix() {
        let r = make_result(&["name", "user_id", "created_at"]);
        assert_eq!(find_pk_idx(&r), Some(1));
    }

    #[test]
    fn find_pk_idx_none() {
        let r = make_result(&["name", "email"]);
        assert_eq!(find_pk_idx(&r), None);
    }

    #[test]
    fn build_pk_where_with_pk_mysql() {
        let r = make_result(&["id", "name"]);
        let row = Row {
            values: vec![Value::Int(7), Value::Text("alice".into())],
        };
        let s = build_pk_where(&r, &row, DriverKind::Mysql);
        assert_eq!(s, "`id` = 7");
    }

    #[test]
    fn build_pk_where_with_pk_postgres() {
        let r = make_result(&["user_id", "name"]);
        let row = Row {
            values: vec![Value::Int(42), Value::Text("bob".into())],
        };
        let s = build_pk_where(&r, &row, DriverKind::Postgres);
        assert_eq!(s, "\"user_id\" = 42");
    }

    #[test]
    fn build_pk_where_fallback_all_columns() {
        let r = make_result(&["name", "email"]);
        let row = Row {
            values: vec![Value::Text("a".into()), Value::Null],
        };
        let s = build_pk_where(&r, &row, DriverKind::Mysql);
        assert_eq!(s, "`name` = 'a' AND `email` = NULL");
    }

    #[test]
    fn dml_row_limit_mysql() {
        assert_eq!(dml_row_limit(DriverKind::Mysql), " LIMIT 1");
    }

    #[test]
    fn dml_row_limit_postgres_empty() {
        assert_eq!(dml_row_limit(DriverKind::Postgres), "");
    }

    #[test]
    fn build_new_value_int_to_int() {
        let v = build_new_value_for_old(&Value::Int(0), "100");
        assert!(matches!(v, Value::Int(100)));
    }

    #[test]
    fn build_new_value_int_to_text_on_parse_fail() {
        let v = build_new_value_for_old(&Value::Int(0), "abc");
        assert_eq!(lit(&v), "'abc'");
    }

    #[test]
    fn build_new_value_null_with_empty_input() {
        let v = build_new_value_for_old(&Value::Null, "");
        assert!(matches!(v, Value::Null));
    }

    #[test]
    fn build_new_value_null_with_text() {
        let v = build_new_value_for_old(&Value::Null, "hello");
        assert_eq!(lit(&v), "'hello'");
    }
}
