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
use super::flatten::build_flat_table_with;

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
        }];
    }

    /// 双击嵌套单元格 → 下钻：数组→元素逐行；对象→单行；标量不下钻。
    /// row_id 是被下钻那一行的 _id（首次下钻=顶层文档 _id），用于记录回写定位上下文
    pub(crate) fn drill_into(
        &mut self,
        field: String,
        row_id: Option<Value>,
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
        let docs = self.drill_stack.last().map(|l| &l.documents)?;
        const MAX_ELEMS: usize = 5000;
        // 逐段穿透路径：object 进入字段，array 展开元素后继续（jobs.connectors）
        let mut current: Vec<&Value> = docs.iter().collect();
        for seg in path.split('.') {
            let mut next: Vec<&Value> = Vec::new();
            for v in &current {
                match v {
                    Value::Object(m) => {
                        if let Some(c) = m.get(seg) {
                            next.push(c);
                        }
                    }
                    Value::Array(arr) => {
                        for el in arr {
                            if let Value::Object(m) = el
                                && let Some(c) = m.get(seg)
                            {
                                next.push(c);
                            }
                        }
                    }
                    _ => {}
                }
            }
            current = next;
        }
        // 终值：array → 元素逐行；object → 一行；标量跳过。裸字段、不加来源
        let mut flat: Vec<Value> = Vec::new();
        for v in current {
            if flat.len() >= MAX_ELEMS {
                break;
            }
            match v {
                Value::Array(arr) => {
                    for el in arr {
                        flat.push(el.clone());
                        if flat.len() >= MAX_ELEMS {
                            break;
                        }
                    }
                }
                Value::Object(_) => flat.push(v.clone()),
                _ => {}
            }
        }
        if flat.is_empty() {
            return None;
        }
        let ft = build_flat_table_with(&flat, &BTreeSet::new());
        Some((Arc::new(flat), Arc::new(ft), path))
    }
}
