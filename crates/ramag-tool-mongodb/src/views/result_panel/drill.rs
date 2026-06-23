//! 嵌套数据原地下钻：双击嵌套单元格 → 把该值当新结果集，复用大列表渲染；面包屑导航返回。
//! 下钻层只读（内嵌数据非独立 collection，编辑需回写父文档，暂不支持）。

use std::collections::BTreeSet;
use std::sync::Arc;

use gpui::{
    App, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, Point,
    SharedString, Styled, Window, div, prelude::*, px,
};
use gpui_component::{ActiveTheme, h_flex};
use ramag_domain::entities::MongoQueryResult;
use serde_json::Value;

use super::FlatTable;
use super::ResultPanel;
use super::cell::{Cell, cell_for_value};
use super::flatten::{Column, build_flat_table_with};

/// 下钻栈一层：label 用于面包屑显示，documents 为该层文档
pub(crate) struct DrillLevel {
    pub label: String,
    pub documents: Vec<Value>,
    /// 顶层文档 _id（回写定位用，一路继承；顶层与无 _id 时为 None）
    pub parent_id: Option<Value>,
    /// 从根到本层的 dotted 路径前缀（如 "project" / "project.sub"；顶层为空）
    pub path_prefix: String,
    /// 本层能否回写编辑：对象下钻=true，数组下钻=false（丢了元素下标）
    pub editable: bool,
    /// 祖先 (对象名, id) 链（根→直接父，本层常量）：作前导列展示，列名即对象名（面包屑里的层级名）
    pub ancestors: Vec<(String, Cell)>,
}

impl ResultPanel {
    /// 是否已下钻（栈深 > 1）→ 显示面包屑（对象层可编辑，数组层只读）
    pub(crate) fn is_drilled(&self) -> bool {
        self.drill_stack.len() > 1
    }

    /// 当前下钻层可否回写编辑：对象下钻层 + 已知顶层 _id
    pub(crate) fn drill_editable(&self) -> bool {
        self.drill_stack
            .last()
            .map(|l| l.editable && l.parent_id.is_some())
            .unwrap_or(false)
    }

    /// 当前下钻层对应的顶层文档 _id（回写 filter 用）
    pub(crate) fn drill_parent_id(&self) -> Option<Value> {
        self.drill_stack.last().and_then(|l| l.parent_id.clone())
    }

    /// 下钻层裸字段 → 完整 dotted 路径（path_prefix.field）
    pub(crate) fn drill_full_path(&self, field: &str) -> String {
        match self.drill_stack.last() {
            Some(l) if !l.path_prefix.is_empty() => format!("{}.{}", l.path_prefix, field),
            _ => field.to_string(),
        }
    }

    /// 重置下钻栈为顶层（新查询时由 set_result 调）
    pub(crate) fn reset_drill(&mut self, label: String, documents: Vec<Value>) {
        self.drill_stack = vec![DrillLevel {
            label,
            documents,
            parent_id: None,
            path_prefix: String::new(),
            editable: false,
            ancestors: Vec::new(),
        }];
    }

    /// 双击嵌套单元格 → 下钻：数组→元素逐行；对象→单行；标量不下钻。
    /// row_id 是被下钻那一行的 _id（首次下钻=顶层文档 _id），用于记录回写定位上下文
    pub(crate) fn drill_into(
        &mut self,
        field: String,
        row_id: Option<Value>,
        row_ident: Option<Value>,
        value: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 对象下钻可回写（顶层 _id + dotted path 定位）；数组下钻丢了元素下标，保持只读
        let editable = matches!(value, Value::Object(_));
        let documents = match value {
            Value::Array(arr) => arr,
            Value::Object(_) => vec![value],
            _ => return,
        };
        let top = self.drill_stack.last();
        // 顶层 _id 一路继承；首次下钻栈顶是顶层文档，用其行 _id
        let parent_id = top.and_then(|l| l.parent_id.clone()).or(row_id);
        // 祖先链：继承父层，再追加"被下钻那一层"的对象名 + 该行 id（_id 或 id），列名即对象名
        let mut ancestors = top.map(|l| l.ancestors.clone()).unwrap_or_default();
        let from_label = top.map(|l| l.label.clone()).unwrap_or_default();
        if let Some(ident) = &row_ident {
            ancestors.push((from_label, cell_for_value(ident)));
        }
        let prefix = top.map(|l| l.path_prefix.clone()).unwrap_or_default();
        let path_prefix = if prefix.is_empty() {
            field.clone()
        } else {
            format!("{prefix}.{field}")
        };
        self.drill_stack.push(DrillLevel {
            label: field,
            documents,
            parent_id,
            path_prefix,
            editable,
            ancestors,
        });
        self.apply_top_level(window, cx);
    }

    /// 点面包屑第 index 层 → 截断栈并恢复该层
    pub(crate) fn drill_to(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index + 1 >= self.drill_stack.len() {
            return;
        }
        self.drill_stack.truncate(index + 1);
        self.apply_top_level(window, cx);
    }

    /// 栈顶 documents → 当前显示：重算表格 + 合成 result + 同步补全源 + 清过滤 + 滚动归零
    fn apply_top_level(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let docs = self
            .drill_stack
            .last()
            .map(|l| l.documents.clone())
            .unwrap_or_default();
        self.selected_rows.clear();
        // 换层清空过滤（新层新列，旧过滤无意义）→ 展开路径随之清空
        self.column_filter
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.row_filter
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.result = Some(MongoQueryResult::read(docs, 0));
        // 重建基础表 + 补全源（过滤已清空）
        self.rebuild_table();
        self.h_scroll.set_offset(Point::new(px(0.0), px(0.0)));
        cx.notify();
    }

    /// 面包屑栏（仅下钻后渲染）：可点段返回上层，当前层高亮，右侧「只读」提示
    pub(crate) fn render_breadcrumb(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let fg = cx.theme().foreground;
        let muted = cx.theme().muted_foreground;
        let secondary = cx.theme().secondary;
        let border = cx.theme().border;
        let last = self.drill_stack.len().saturating_sub(1);

        let mut bar = h_flex()
            .w_full()
            .flex_none()
            .px_3()
            .py(px(5.0))
            .gap_1()
            .items_center()
            .bg(secondary)
            .border_b_1()
            .border_color(border)
            .text_xs();
        for (i, level) in self.drill_stack.iter().enumerate() {
            if i > 0 {
                bar = bar.child(div().text_color(muted).child(SharedString::from("›")));
            }
            let label = SharedString::from(level.label.clone());
            if i == last {
                bar = bar.child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .child(label),
                );
            } else {
                bar = bar.child(
                    div()
                        .id(SharedString::from(format!("mongo-bc-{i}")))
                        .cursor_pointer()
                        .text_color(muted)
                        .hover(move |s| s.text_color(fg))
                        .child(label)
                        .on_click(
                            cx.listener(move |panel, _, window, cx| panel.drill_to(i, window, cx)),
                        ),
                );
            }
        }
        let bar = bar.child(div().flex_1());
        // 对象下钻层可改字段；数组层 / 无 _id 层仍只读，右侧提示用户当前层能力
        if self.drill_editable() {
            bar.child(div().text_color(muted).child(SharedString::from("可编辑")))
        } else {
            bar.child(div().text_color(muted).child(SharedString::from("只读")))
        }
    }

    /// 输入对象/数组路径 → 钻进去（逐段穿透数组）：终值 object 一行 / array 元素逐行，裸字段。
    /// 返回 (钻取文档, 钻取表, 路径)；非钻取路径返回 None
    pub(crate) fn try_drill_path(
        &self,
        cx: &App,
    ) -> Option<(Arc<Vec<Value>>, Arc<FlatTable>, String)> {
        let path = self.parse_column_filter(cx).drill_path?;
        let level = self.drill_stack.last()?;
        let docs = &level.documents;
        const MAX_ELEMS: usize = 5000;
        // 逐段穿透并携带每行的 (对象名, id) 祖先链：从当前层已有祖先起步。
        // node_label = 正被穿过的那层对象名（首层 = 当前 drill 层名，之后 = 上一路径段）
        let base: Vec<(String, Cell)> = level.ancestors.clone();
        let mut node_label = level.label.clone();
        let mut current: Vec<(Vec<(String, Cell)>, &Value)> =
            docs.iter().map(|d| (base.clone(), d)).collect();
        for seg in path.split('.') {
            let mut next: Vec<(Vec<(String, Cell)>, &Value)> = Vec::new();
            for entry in &current {
                let anc = &entry.0;
                let v: &Value = entry.1;
                match v {
                    Value::Object(m) => {
                        let mut a = anc.clone();
                        a.push((node_label.clone(), id_cell_of(m)));
                        if let Some(c) = m.get(seg) {
                            next.push((a, c));
                        }
                    }
                    Value::Array(arr) => {
                        for el in arr {
                            if let Value::Object(m) = el {
                                let mut a = anc.clone();
                                a.push((node_label.clone(), id_cell_of(m)));
                                if let Some(c) = m.get(seg) {
                                    next.push((a, c));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            node_label = seg.to_string();
            current = next;
        }
        // 终值：array → 元素逐行；object → 一行；标量跳过。祖先链随行一并带出
        let mut rows: Vec<(Vec<(String, Cell)>, Value)> = Vec::new();
        for (anc, v) in current {
            if rows.len() >= MAX_ELEMS {
                break;
            }
            match v {
                Value::Array(arr) => {
                    for el in arr {
                        rows.push((anc.clone(), el.clone()));
                        if rows.len() >= MAX_ELEMS {
                            break;
                        }
                    }
                }
                Value::Object(_) => rows.push((anc, v.clone())),
                _ => {}
            }
        }
        if rows.is_empty() {
            return None;
        }
        let flat: Vec<Value> = rows.iter().map(|(_, v)| v.clone()).collect();
        let anc_rows: Vec<Vec<(String, Cell)>> = rows.into_iter().map(|(a, _)| a).collect();
        let mut ft = build_flat_table_with(&flat, &BTreeSet::new());
        prepend_ancestor_columns(&mut ft, &anc_rows);
        Some((Arc::new(flat), Arc::new(ft), path))
    }
}

/// 取对象的标识 id 作 cell：优先 `_id`，否则 `id`；都无则空 cell
fn id_cell_of(m: &serde_json::Map<String, Value>) -> Cell {
    if let Some(v) = m.get("_id").or_else(|| m.get("id")) {
        cell_for_value(v)
    } else {
        Cell {
            text: String::new(),
            kind: "null",
        }
    }
}

/// 给钻取表加「祖先」前导列：每层一列、根→深保序，列名即对象名；整列为空的层（中间无 id 的对象）丢弃
fn prepend_ancestor_columns(ft: &mut FlatTable, anc_rows: &[Vec<(String, Cell)>]) {
    let depth = anc_rows.iter().map(|a| a.len()).max().unwrap_or(0);
    if depth == 0 {
        return;
    }
    let empty = Cell {
        text: String::new(),
        kind: "null",
    };
    let mut lead_cols: Vec<Column> = Vec::new();
    let mut keep: Vec<usize> = Vec::new();
    for layer in 0..depth {
        let nonempty = anc_rows.iter().any(|a| {
            a.get(layer)
                .map(|(_, c)| !c.text.is_empty())
                .unwrap_or(false)
        });
        if !nonempty {
            continue;
        }
        // 该层对象名（各行一致，取首个出现的）作列名
        let label = anc_rows
            .iter()
            .find_map(|a| a.get(layer))
            .map(|(l, _)| l.clone())
            .unwrap_or_default();
        let kind = anc_rows
            .iter()
            .filter_map(|a| a.get(layer))
            .find(|(_, c)| c.kind != "null")
            .map(|(_, c)| c.kind)
            .unwrap_or("text");
        lead_cols.push(Column { path: label, kind });
        keep.push(layer);
    }
    if lead_cols.is_empty() {
        return;
    }
    let lead_rows: Vec<Vec<Cell>> = anc_rows
        .iter()
        .map(|a| {
            keep.iter()
                .map(|&l| {
                    a.get(l)
                        .map(|(_, c)| c.clone())
                        .unwrap_or_else(|| empty.clone())
                })
                .collect()
        })
        .collect();
    ft.prepend_lead(lead_cols, lead_rows);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(v: Value) -> serde_json::Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => unreachable!("expect object"),
        }
    }

    #[test]
    fn id_cell_prefers_id_then_id_field() {
        // _id（$oid 包装）优先，解出裸 id
        let c = id_cell_of(&obj(
            json!({"_id": {"$oid": "507f1f77bcf86cd799439011"}, "x": 1}),
        ));
        assert_eq!(c.text, "507f1f77bcf86cd799439011");
        // 无 _id 时退回 id 字段
        assert_eq!(id_cell_of(&obj(json!({"id": "uuid-123"}))).text, "uuid-123");
        // 都没有 → 空
        assert!(id_cell_of(&obj(json!({"x": 1}))).text.is_empty());
    }

    #[test]
    fn ancestor_columns_use_object_name_and_drop_empty_layer() {
        let mut ft = build_flat_table_with(
            &[json!({"a": 1}), json!({"a": 2})],
            &std::collections::BTreeSet::new(),
        );
        // 层0 对象名 "root" 都有 id；层1 "mid" 全空（中间无 id）→ 应丢弃层1
        let anc_rows = vec![
            vec![
                (
                    "root".to_string(),
                    Cell {
                        text: "t1".to_string(),
                        kind: "text",
                    },
                ),
                (
                    "mid".to_string(),
                    Cell {
                        text: String::new(),
                        kind: "null",
                    },
                ),
            ],
            vec![
                (
                    "root".to_string(),
                    Cell {
                        text: "t2".to_string(),
                        kind: "text",
                    },
                ),
                (
                    "mid".to_string(),
                    Cell {
                        text: String::new(),
                        kind: "null",
                    },
                ),
            ],
        ];
        prepend_ancestor_columns(&mut ft, &anc_rows);
        // 列名即对象名（不是 ‹父N›）；空层 "mid" 被丢弃
        assert_eq!(ft.columns[0].path, "root");
        assert!(!ft.columns.iter().any(|c| c.path == "mid"));
        assert_eq!(ft.rows[0][0].text, "t1");
        assert_eq!(ft.rows[1][0].text, "t2");
    }
}
