//! MongoDB 编辑器与过滤框补全：实现 gpui-component CompletionProvider。
//! - CommandCompletionProvider：JSON 命令编辑器（命令名 / 参数名 / 查询聚合操作符）
//! - ColumnFilterCompletionProvider：结果区"过滤列"（候选为当前结果集列 path，逗号分隔）

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

/// runCommand 顶层命令名 + 常用参数名（不带 $）
const MONGO_COMMANDS: &[&str] = &[
    "find",
    "aggregate",
    "count",
    "distinct",
    "insert",
    "update",
    "delete",
    "findAndModify",
    "getMore",
    "listCollections",
    "listIndexes",
    "createIndexes",
    "dropIndexes",
    "drop",
    "create",
    "renameCollection",
    "ping",
    "dbStats",
    "collStats",
    "serverStatus",
    "filter",
    "projection",
    "sort",
    "limit",
    "skip",
    "pipeline",
    "query",
    "documents",
    "updates",
    "deletes",
    "cursor",
    "batchSize",
    "hint",
    "collation",
    "new",
    "upsert",
    "multi",
    "ordered",
];

/// 查询 / 聚合操作符（带 $ 前缀）
const MONGO_OPERATORS: &[&str] = &[
    "$eq",
    "$ne",
    "$gt",
    "$gte",
    "$lt",
    "$lte",
    "$in",
    "$nin",
    "$and",
    "$or",
    "$not",
    "$nor",
    "$exists",
    "$type",
    "$regex",
    "$expr",
    "$mod",
    "$text",
    "$where",
    "$all",
    "$elemMatch",
    "$size",
    "$match",
    "$group",
    "$project",
    "$sort",
    "$limit",
    "$skip",
    "$unwind",
    "$lookup",
    "$count",
    "$facet",
    "$addFields",
    "$set",
    "$unset",
    "$sortByCount",
    "$sample",
    "$sum",
    "$avg",
    "$min",
    "$max",
    "$first",
    "$last",
    "$push",
    "$addToSet",
    "$concat",
    "$cond",
    "$ifNull",
    "$dateToString",
];

/// 单条补全项：InsertAndReplace 保证覆盖已输入的前缀
fn make_item(
    label: &str,
    kind: CompletionItemKind,
    detail: &str,
    range: lsp_types::Range,
) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail: Some(detail.to_string()),
        text_edit: Some(CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
            new_text: label.to_string(),
            insert: range,
            replace: range,
        })),
        ..Default::default()
    }
}

/// 取光标前的补全前缀（字母 / 数字 / _ / $），返回 (起点字节 offset, 前缀)
fn word_prefix(text: &str, offset: usize) -> (usize, &str) {
    let bytes = text.as_bytes();
    let end = offset.min(bytes.len());
    let mut start = end;
    while start > 0 {
        let b = bytes[start - 1];
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'$' {
            start -= 1;
        } else {
            break;
        }
    }
    (start, &text[start..end])
}

/// 命令编辑器补全：`$` 前缀补操作符，否则补命令名 / 参数名
pub struct CommandCompletionProvider;

impl CommandCompletionProvider {
    pub fn new_rc() -> Rc<dyn CompletionProvider> {
        Rc::new(Self)
    }
}

impl CompletionProvider for CommandCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        _cx: &mut Context<InputState>,
    ) -> Task<Result<CompletionResponse>> {
        let text = rope.to_string();
        let real_offset = offset.min(text.len());
        let (start, prefix) = word_prefix(&text, real_offset);
        if prefix.is_empty() {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        }
        let prefix_lower = prefix.to_ascii_lowercase();
        let range = lsp_types::Range::new(
            rope.offset_to_position(start),
            rope.offset_to_position(real_offset),
        );

        let mut items: Vec<CompletionItem> = Vec::new();
        if prefix.starts_with('$') {
            for op in MONGO_OPERATORS {
                if op.to_ascii_lowercase().starts_with(&prefix_lower) {
                    items.push(make_item(
                        op,
                        CompletionItemKind::OPERATOR,
                        "operator",
                        range,
                    ));
                }
            }
        } else {
            for kw in MONGO_COMMANDS {
                if kw.to_ascii_lowercase().starts_with(&prefix_lower) {
                    items.push(make_item(kw, CompletionItemKind::KEYWORD, "command", range));
                }
            }
        }
        items.truncate(50);
        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(
        &self,
        _offset: usize,
        new_text: &str,
        _cx: &mut Context<InputState>,
    ) -> bool {
        new_text
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    }
}

/// "过滤列"补全：候选为当前结果集列 path，按逗号切 token 仅匹配最后一段（与 dbclient 同款）
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
        // token 起点：向前扫到最近逗号，再跳过前导空格
        let mut tok_start = real_offset;
        while tok_start > 0 && bytes[tok_start - 1] != b',' && bytes[tok_start - 1] != b';' {
            tok_start -= 1;
        }
        while tok_start < real_offset && bytes[tok_start] == b' ' {
            tok_start += 1;
        }
        let prefix = &text[tok_start..real_offset];
        if prefix.is_empty() {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        }
        let prefix_lower = prefix.to_ascii_lowercase();
        let range = lsp_types::Range::new(
            rope.offset_to_position(tok_start),
            rope.offset_to_position(real_offset),
        );

        // 已填入的其它列不再建议，避免重复
        let already: std::collections::HashSet<String> = text
            .split([',', ';'])
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty() && *s != prefix_lower)
            .collect();

        let cols = self.columns.read();
        let mut items: Vec<CompletionItem> = Vec::new();
        // 光标在分号后 → 投影：候选 = 钻取路径（分号前最后一个 token）下的子字段（裸名）
        let drill = text[..real_offset]
            .rsplit_once(';')
            .map(|(head, _)| {
                head.rsplit(',')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase()
            })
            .filter(|d| !d.is_empty());
        if let Some(drill) = drill {
            let pfx = format!("{drill}.");
            let mut seen = std::collections::HashSet::new();
            for name in cols.iter() {
                let lc = name.to_ascii_lowercase();
                let Some(rest) = lc.strip_prefix(&pfx) else {
                    continue;
                };
                let seg_lc = rest.split('.').next().unwrap_or(rest).to_string();
                if seg_lc.is_empty()
                    || !seg_lc.contains(&prefix_lower)
                    || already.contains(&seg_lc)
                    || !seen.insert(seg_lc.clone())
                {
                    continue;
                }
                // 原始大小写：从 name 去掉同长前缀取首段（ASCII 大小写不改字节长度）
                let orig_seg = name[pfx.len()..].split('.').next().unwrap_or("");
                items.push(make_item(
                    orig_seg,
                    CompletionItemKind::FIELD,
                    "field",
                    range,
                ));
                if items.len() >= 50 {
                    break;
                }
            }
        } else if prefix.contains('.') {
            // 点号深入（分号前 = 展开条件）：只提示能再展开的对象/数组（有更深子路径），标量叶子不提示
            let last_dot = prefix_lower.rfind('.').unwrap_or(0);
            let child_prefix = prefix_lower[..=last_dot].to_string(); // "jobs."
            let seg_prefix = &prefix_lower[last_dot + 1..]; // "" 或 "con"
            let mut seen = std::collections::HashSet::new();
            for name in cols.iter() {
                let lc = name.to_ascii_lowercase();
                let Some(rest) = lc.strip_prefix(&child_prefix) else {
                    continue;
                };
                let seg = rest.split('.').next().unwrap_or(rest);
                // 可展开 ⟺ 该子字段还有更深子路径（object / array-of-object）；标量叶子跳过
                if rest.len() <= seg.len()
                    || seg.is_empty()
                    || !seg.contains(seg_prefix)
                    || !seen.insert(seg.to_string())
                {
                    continue;
                }
                let full = &name[..child_prefix.len() + seg.len()];
                items.push(make_item(full, CompletionItemKind::FIELD, "object", range));
                if items.len() >= 50 {
                    break;
                }
            }
        } else {
            // 无点：只提示顶层字段名（各路径第一段，去重），子串匹配；打点后才深入子字段
            let mut seen = std::collections::HashSet::new();
            for name in cols.iter() {
                let top = name.split('.').next().unwrap_or(name);
                let lc = top.to_ascii_lowercase();
                if !lc.contains(&prefix_lower) || already.contains(&lc) || !seen.insert(lc.clone())
                {
                    continue;
                }
                items.push(make_item(top, CompletionItemKind::FIELD, "column", range));
                if items.len() >= 50 {
                    break;
                }
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
        // 字母 / 数字 / 下划线 / 点号触发（点号用于深入嵌套 consume.子字段）；逗号不触发
        new_text
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_prefix_includes_dollar_and_stops_at_quote() {
        assert_eq!(word_prefix("{\"$gt", 5).1, "$gt");
        assert_eq!(word_prefix("\"find", 5).1, "find");
        assert_eq!(word_prefix("a, b", 1).1, "a");
        assert_eq!(word_prefix("{ ", 2).1, "");
    }
}
