//! 嵌套数据原地下钻：双击嵌套单元格 → 把该值当新结果集，复用大列表渲染；面包屑导航返回。
//! 下钻层只读（内嵌数据非独立 collection，编辑需回写父文档，暂不支持）。

use std::sync::Arc;

use gpui::{
    App, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement, Point,
    SharedString, Styled, Window, div, prelude::*, px,
};
use gpui_component::{ActiveTheme, h_flex};
use ramag_domain::entities::MongoQueryResult;
use serde_json::{Map, Value, json};

use super::FlatTable;
use super::ResultPanel;
use super::flatten::build_flat_table;

/// 下钻栈一层：label 用于面包屑显示，documents 为该层文档
pub(crate) struct DrillLevel {
    pub label: String,
    pub documents: Vec<Value>,
}

impl ResultPanel {
    /// 是否已下钻（栈深 > 1）→ 只读 + 显示面包屑
    pub(crate) fn is_drilled(&self) -> bool {
        self.drill_stack.len() > 1
    }

    /// 重置下钻栈为顶层（新查询时由 set_result 调）
    pub(crate) fn reset_drill(&mut self, label: String, documents: Vec<Value>) {
        self.drill_stack = vec![DrillLevel { label, documents }];
    }

    /// 双击嵌套单元格 → 下钻：数组→元素逐行；对象→单行；标量不下钻
    pub(crate) fn drill_into(
        &mut self,
        label: String,
        value: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let documents = match value {
            Value::Array(arr) => arr,
            Value::Object(_) => vec![value],
            _ => return,
        };
        self.drill_stack.push(DrillLevel { label, documents });
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
        let table = if docs.is_empty() {
            None
        } else {
            Some(Arc::new(build_flat_table(&docs)))
        };
        match &table {
            Some(t) => {
                *self.column_completion_source.write() =
                    t.columns.iter().map(|c| c.path.clone()).collect()
            }
            None => self.column_completion_source.write().clear(),
        }
        self.table = table;
        self.result = Some(MongoQueryResult::read(docs, 0));
        // 换层清空过滤（新层新列，旧过滤无意义）+ 滚动归位
        self.column_filter
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.row_filter
            .update(cx, |s, cx| s.set_value("", window, cx));
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
        bar.child(div().flex_1())
            .child(div().text_color(muted).child(SharedString::from("只读")))
    }

    /// 列过滤后只剩 1 个数组/对象列 → 展平汇总：所有行该列元素合并成新结果，加「来源」列（原行号）。
    /// 返回 (展平文档, 展平表, 列名)；不满足返回 None
    pub(crate) fn try_flatten_single_column(
        &self,
        cx: &App,
    ) -> Option<(Arc<Vec<Value>>, Arc<FlatTable>, String)> {
        let col_indices = self.filtered_column_indices(cx)?;
        if col_indices.len() != 1 {
            return None;
        }
        let table = self.table.as_ref()?;
        let col = table.columns.get(col_indices[0])?;
        if !matches!(col.kind, "array" | "object") {
            return None;
        }
        let result = self.result.as_ref()?;
        let path = col.path.clone();
        // 上限保护：超大结果集 unwind 截断，避免一次铺开过多
        const MAX_ELEMS: usize = 5000;
        let mut flat: Vec<Value> = Vec::new();
        for (i, doc) in result.documents.iter().enumerate() {
            if flat.len() >= MAX_ELEMS {
                break;
            }
            match doc.get(&path) {
                Some(Value::Array(arr)) => {
                    for el in arr {
                        flat.push(with_source(el, i + 1));
                        if flat.len() >= MAX_ELEMS {
                            break;
                        }
                    }
                }
                Some(v @ Value::Object(_)) => flat.push(with_source(v, i + 1)),
                _ => {}
            }
        }
        if flat.is_empty() {
            return None;
        }
        let mut ft = build_flat_table(&flat);
        move_source_first(&mut ft);
        Some((Arc::new(flat), Arc::new(ft), path))
    }
}

/// 来源列名（标元素来自原第几行）
const SOURCE_COL: &str = "来源";

/// 元素 → 对象 + 来源列；标量元素包成 {值, 来源}
fn with_source(el: &Value, src: usize) -> Value {
    let mut m = match el {
        Value::Object(o) => o.clone(),
        other => {
            let mut map = Map::new();
            map.insert("值".to_string(), other.clone());
            map
        }
    };
    m.insert(SOURCE_COL.to_string(), json!(src));
    Value::Object(m)
}

/// 把「来源」列移到最前（build_flat_table 默认按字段名排，来源会沉到中间）
fn move_source_first(ft: &mut FlatTable) {
    if let Some(pos) = ft.columns.iter().position(|c| c.path == SOURCE_COL)
        && pos != 0
    {
        ft.columns[..=pos].rotate_right(1);
        for row in &mut ft.rows {
            row[..=pos].rotate_right(1);
        }
    }
}
