//! 结果展示：表格视图（只解析第一层字段；嵌套对象/数组单元格摘要化，双击看完整）
//! - 顶部 toolbar：过滤列 + 过滤行 + 增删 + 导出
//! - 主体：uniform_list 行级虚拟化的表格（列头 + 行）
//! - 单元格双击：标量编辑 / 嵌套看完整 JSON

mod drill;
mod flatten;
mod ops;
mod table;
mod toolbar;

use std::collections::BTreeSet;
use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement, Point, Render,
    ScrollHandle, SharedString, Styled, UniformListScrollHandle, Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Sizable as _, WindowExt as _,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use parking_lot::RwLock;
use ramag_app::MongoService;
use ramag_domain::entities::{ConnectionConfig, MongoQueryResult};
use serde_json::Value;

pub use flatten::FlatTable;

/// 过滤列补全收集的最大嵌套深度（支持 consume.detail.x 这类多层）
const PATH_COMPLETION_DEPTH: usize = 5;

pub struct ResultPanel {
    pub(crate) result: Option<MongoQueryResult>,
    pub(crate) error: Option<String>,
    pub(crate) running: bool,
    /// 扁平化表格视图（result 变化时重算）
    pub(crate) table: Option<Arc<FlatTable>>,
    /// 工具栏：过滤列名（逗号分隔多列）
    pub(crate) column_filter: Entity<InputState>,
    /// 工具栏：过滤行（子串包含；任意单元格匹配即保留）
    pub(crate) row_filter: Entity<InputState>,
    /// 表格虚拟列表纵滚句柄（uniform_list 内部 Y）
    pub(crate) uniform_scroll: UniformListScrollHandle,
    /// 横滚句柄（外层 div X；与 dbclient::result_table 同模式）
    pub(crate) h_scroll: ScrollHandle,
    /// 列过滤框补全候选源（set_result 时填入当前结果集列 path）
    pub(crate) column_completion_source: Arc<RwLock<Vec<String>>>,
    /// DML 执行上下文（由 query_tab 注入；None 时禁用增删改）
    pub(crate) service: Option<Arc<MongoService>>,
    pub(crate) config: Option<ConnectionConfig>,
    pub(crate) database: String,
    /// 当前结果对应的 collection（run 时从命令提取，是增删改的目标）
    pub(crate) target_collection: Option<String>,
    /// 异步 DML 完成后挂起的 toast，下次 render 推送
    pub(crate) pending_notification: Option<gpui_component::notification::Notification>,
    /// 勾选的行（按 documents 索引）；删除文档用
    pub(crate) selected_rows: BTreeSet<usize>,
    /// 下钻栈：栈底=原始查询结果，双击嵌套 push 一层；栈深 > 1 即下钻态（只读 + 面包屑）
    pub(crate) drill_stack: Vec<drill::DrillLevel>,
    /// 当前排序列 path + 方向；用 path 而非索引，钻取换表后失配自动失效
    pub(crate) sort_by: Option<(String, SortDir)>,
    _subscriptions: Vec<gpui::Subscription>,
}

/// 结果区事件：DML 成功后请求 query_tab 重跑当前命令以刷新结果
#[derive(Clone, Debug)]
pub enum ResultEvent {
    Refresh,
}

/// 排序方向（单击列头切换 None→Asc→Desc→None）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SortDir {
    Asc,
    Desc,
}

impl EventEmitter<ResultEvent> for ResultPanel {}

impl ResultPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // 列过滤补全源：set_result 时填入当前结果集列 path，供 ColumnFilterCompletionProvider 读取
        let column_completion_source: Arc<RwLock<Vec<String>>> = Arc::new(RwLock::new(Vec::new()));
        let provider = crate::completion::ColumnFilterCompletionProvider::new_rc(
            column_completion_source.clone(),
        );
        let column_filter = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("过滤列（逗号分隔多列名）");
            state.lsp.completion_provider = Some(provider);
            state
        });
        let row_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("过滤行（任意单元格包含）"));

        let subs = vec![
            cx.subscribe(&column_filter, |_this, _, _e: &InputEvent, cx| {
                // 钻取/投影在 render 时派生（基础表不变）；补全源在 rebuild 时已就绪，仅重渲染
                cx.notify();
            }),
            cx.subscribe(&row_filter, |_this, _, _e: &InputEvent, cx| cx.notify()),
        ];

        Self {
            result: None,
            error: None,
            running: false,
            table: None,
            column_filter,
            row_filter,
            uniform_scroll: UniformListScrollHandle::new(),
            h_scroll: ScrollHandle::new(),
            column_completion_source,
            service: None,
            config: None,
            database: String::new(),
            target_collection: None,
            pending_notification: None,
            selected_rows: BTreeSet::new(),
            drill_stack: Vec::new(),
            sort_by: None,
            _subscriptions: subs,
        }
    }

    /// 注入 DML 执行上下文（连接 + 默认库）；由 query_tab::new 调
    pub fn set_context(
        &mut self,
        service: Arc<MongoService>,
        config: ConnectionConfig,
        database: String,
    ) {
        self.service = Some(service);
        self.config = Some(config);
        self.database = database;
    }

    /// 设置当前结果对应的 collection（增删改目标）；run 提取命令后调
    pub fn set_target_collection(&mut self, coll: Option<String>) {
        self.target_collection = coll;
    }

    /// 同步当前 db：切库 / 切 collection 后写操作（update/delete/insert）要落到正确的库，
    /// 否则沿用 tab 初始库、filter 匹配不到文档（matched 0）→ 表现为「更新不了」
    pub fn set_database(&mut self, db: String) {
        self.database = db;
    }

    /// 清空列 / 行过滤框：切换 collection（换数据源）时调，避免旧过滤词残留串到新结果
    pub fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.column_filter
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.row_filter
            .update(cx, |s, cx| s.set_value("", window, cx));
    }

    /// 能否写（增删改）：上下文齐全 + 有目标 collection
    pub(crate) fn can_write(&self) -> bool {
        self.service.is_some() && self.config.is_some() && self.target_collection.is_some()
    }

    /// 切换某行勾选（按 documents 索引）
    pub(crate) fn toggle_row(&mut self, idx: usize, cx: &mut Context<Self>) {
        if !self.selected_rows.insert(idx) {
            self.selected_rows.remove(&idx);
        }
        cx.notify();
    }

    /// 全选 / 全不选（传入当前可见的全部行索引）
    pub(crate) fn toggle_all(&mut self, all: &[usize], cx: &mut Context<Self>) {
        if !all.is_empty() && all.iter().all(|i| self.selected_rows.contains(i)) {
            for i in all {
                self.selected_rows.remove(i);
            }
        } else {
            self.selected_rows.extend(all.iter().copied());
        }
        cx.notify();
    }

    pub(crate) fn is_row_selected(&self, idx: usize) -> bool {
        self.selected_rows.contains(&idx)
    }

    /// 单击列头切换排序：同列 None→Asc→Desc→None；换列直接 Asc
    pub(crate) fn toggle_sort(&mut self, path: String, cx: &mut Context<Self>) {
        self.sort_by = match self.sort_by.take() {
            Some((p, SortDir::Asc)) if p == path => Some((path, SortDir::Desc)),
            Some((p, SortDir::Desc)) if p == path => None,
            _ => Some((path, SortDir::Asc)),
        };
        cx.notify();
    }

    pub fn set_running(&mut self, cx: &mut Context<Self>) {
        self.running = true;
        self.error = None;
        cx.notify();
    }

    pub fn set_result(&mut self, r: MongoQueryResult, cx: &mut Context<Self>) {
        self.selected_rows.clear();
        // 新查询：重置下钻栈为顶层（label 用目标 collection）
        let label = self
            .target_collection
            .clone()
            .unwrap_or_else(|| "结果".to_string());
        self.reset_drill(label, r.documents.clone());
        self.sort_by = None;
        self.result = Some(r);
        self.error = None;
        self.running = false;
        // 切结果时把横滚归位最左（与 dbclient::result_table 同款），避免新表格沿用旧的横滚 X 位置
        self.h_scroll.set_offset(Point::new(px(0.0), px(0.0)));
        // 建基础表 + 刷新补全源
        self.rebuild_table();
        cx.notify();
    }

    pub fn set_error(&mut self, err: String, cx: &mut Context<Self>) {
        self.error = Some(err);
        self.running = false;
        cx.notify();
    }

    /// 解析过滤列框（结合当前层 docs 判字段类型）；规则见 classify_filter
    pub(crate) fn parse_column_filter(&self, cx: &gpui::App) -> ParsedFilter {
        let raw = self.column_filter.read(cx).value().to_string();
        let docs = self
            .drill_stack
            .last()
            .map(|l| l.documents.as_slice())
            .unwrap_or(&[]);
        classify_filter(&raw, docs)
    }

    /// 重建基础表格（不钻取）与补全源；钻取/投影在 render 时按过滤框派生
    pub(crate) fn rebuild_table(&mut self) {
        let docs = self
            .drill_stack
            .last()
            .map(|l| l.documents.clone())
            .unwrap_or_default();
        self.table = if docs.is_empty() {
            None
        } else {
            Some(Arc::new(flatten::build_flat_table_with(
                &docs,
                &BTreeSet::new(),
            )))
        };
        *self.column_completion_source.write() =
            flatten::collect_paths(&docs, PATH_COMPLETION_DEPTH);
    }

    /// 当前过滤后的列索引（None 表示全选）；用所有 token 子串匹配（unwind 锚不匹配子列、自然不影响）
    pub(crate) fn filtered_column_indices(&self, cx: &gpui::App) -> Option<Vec<usize>> {
        column_indices_for(self.table.as_ref()?, &self.parse_column_filter(cx).filters)
    }

    /// 当前过滤后的行索引；空过滤串等价 None
    pub(crate) fn filtered_row_indices(&self, cx: &gpui::App) -> Option<Vec<usize>> {
        let raw = self.row_filter.read(cx).value().to_string();
        row_indices_for(self.table.as_ref()?, &raw)
    }

    /// 双击单元格 → 弹该单元格内容详情（与 MySQL dbclient::cell_edit_dialog 同款交互）。
    /// 标题是「{列名} ({类型})」；内容若是合法 JSON 自动 pretty 格式化
    pub(crate) fn open_cell_dialog(
        &self,
        column_path: String,
        kind: &'static str,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let display = if text.starts_with('{') || text.starts_with('[') {
            serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| serde_json::to_string_pretty(&v).ok())
                .unwrap_or_else(|| text.clone())
        } else {
            text.clone()
        };
        let title: SharedString = SharedString::from(format!("{column_path}  ({kind})"));
        let input: Entity<InputState> = cx.new(|cx_inner| {
            InputState::new(window, cx_inner)
                .multi_line(true)
                .default_value(display)
        });
        window.open_dialog(cx, move |dialog, _w, _app| {
            let input = input.clone();
            let title = title.clone();
            dialog
                .title(title)
                .close_button(true)
                .w(px(720.0))
                .p(px(20.0))
                .content(move |content, _, _| {
                    content.child(
                        div()
                            .w_full()
                            .h(px(400.0))
                            .child(Input::new(&input).small().h_full().disabled(true)),
                    )
                })
        });
    }
}

impl Render for ResultPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 异步 DML 完成的 toast 在这里推送
        if let Some(n) = self.pending_notification.take() {
            window.push_notification(n, cx);
        }
        // 把需要的颜色字段提前 Copy 出来，避免 cx.theme() 的 immut borrow 与 toolbar/table 的 mut borrow 冲突
        let bg = cx.theme().background;
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;

        if self.running {
            return v_flex()
                .size_full()
                .bg(bg)
                .child(toolbar::render(self, cx))
                .child(empty_hint("执行中…", muted))
                .into_any_element();
        }
        if let Some(err) = self.error.clone() {
            return v_flex()
                .size_full()
                .bg(bg)
                .child(toolbar::render(self, cx))
                .child(error_hint(err, danger))
                .into_any_element();
        }
        let Some(result) = self.result.clone() else {
            return v_flex()
                .size_full()
                .bg(bg)
                .child(toolbar::render(self, cx))
                .child(empty_hint(
                    "（点击左侧 collection 自动开 Tab，或在编辑器写命令后运行）",
                    muted,
                ))
                .into_any_element();
        };
        let Some(table_arc) = self.table.clone() else {
            let hint = if result.affected > 0 {
                format!("已执行写操作，影响 {} 条", result.affected)
            } else if self.is_drilled() {
                "（空）".to_string()
            } else {
                "（无文档返回）".to_string()
            };
            // 下钻到空数据也要保留面包屑（toolbar 下方），否则无法返回上层
            let mut root = v_flex().size_full().bg(bg).child(toolbar::render(self, cx));
            if self.is_drilled() {
                root = root.child(self.render_breadcrumb(cx));
            }
            return root.child(empty_hint(hint, muted)).into_any_element();
        };

        // 钻取视图：输入对象/数组路径 → 钻进去只看其字段（裸名）
        if let Some((flat_docs, flat_table, col_path)) = self.try_drill_path(cx) {
            let n = flat_docs.len();
            // 展平汇总视图同样支持列/行过滤（分号后的过滤 token 作用在展平表上）
            let row_q = self.row_filter.read(cx).value().to_string();
            let fcol = column_indices_for(&flat_table, &self.parse_column_filter(cx).filters);
            let frow = row_indices_for(&flat_table, &row_q);
            let mut root = v_flex()
                .size_full()
                .bg(bg)
                .child(toolbar::render(self, cx))
                .child(div().h(px(1.0)).bg(border))
                .child(flatten_hint(&col_path, n, border, muted, bg));
            if self.is_drilled() {
                root = root.child(self.render_breadcrumb(cx));
            }
            return root
                .child(div().flex_1().min_h_0().child(table::render(
                    self,
                    flat_table,
                    fcol,
                    frow,
                    Some(flat_docs),
                    false,
                    cx,
                )))
                .child(render_status_bar(
                    format!("钻取「{col_path}」· {n} 条"),
                    border,
                    muted,
                    bg,
                ))
                .into_any_element();
        }

        let total_docs = result.documents.len();
        let elapsed = result.elapsed_ms;
        let col_indices = self.filtered_column_indices(cx);
        let row_indices = self.filtered_row_indices(cx);
        let filtered_rows = row_indices.as_ref().map(|v| v.len()).unwrap_or(total_docs);
        let total_cols = self.table.as_ref().map(|t| t.columns.len()).unwrap_or(0);
        let visible_cols_count = col_indices.as_ref().map(|v| v.len()).unwrap_or(total_cols);
        let summary = match (row_indices.is_some(), col_indices.is_some()) {
            (true, true) => format!(
                "命中 {visible_cols_count} / {total_cols} 列 · {filtered_rows} / {total_docs} 行 · 耗时 {elapsed}ms"
            ),
            (true, false) => format!("命中 {filtered_rows} / {total_docs} 行 · 耗时 {elapsed}ms"),
            (false, true) => format!(
                "命中 {visible_cols_count} / {total_cols} 列 · {total_docs} 行 · 耗时 {elapsed}ms"
            ),
            (false, false) => format!("{total_docs} 行 · 耗时 {elapsed}ms"),
        };

        // toolbar（搜索栏）始终在顶、位置不变；下钻态时在其下方插入面包屑栏
        let mut root = v_flex()
            .size_full()
            .bg(bg)
            .child(toolbar::render(self, cx))
            .child(div().h(px(1.0)).bg(border));
        if self.is_drilled() {
            root = root.child(self.render_breadcrumb(cx));
        }
        root.child(div().flex_1().min_h_0().child(table::render(
            self,
            table_arc,
            col_indices,
            row_indices,
            None,
            true,
            cx,
        )))
        .child(render_status_bar(summary, border, muted, bg))
        .into_any_element()
    }
}

/// 底部 status bar：行数 / 耗时 / 过滤命中数（与 dbclient::result_table 同款）
fn render_status_bar(
    summary: String,
    border: gpui::Hsla,
    muted: gpui::Hsla,
    bg: gpui::Hsla,
) -> impl IntoElement {
    div()
        .id("mongo-status-bar")
        .w_full()
        .flex_none()
        .px(px(12.0))
        .py(px(4.0))
        .border_t_1()
        .border_color(border)
        .bg(bg)
        .text_xs()
        .text_color(muted)
        .child(SharedString::from(summary))
}

/// 展平视图顶部提示条：已展平某列 + 元素数 + 恢复方式
fn flatten_hint(
    col: &str,
    n: usize,
    border: gpui::Hsla,
    muted: gpui::Hsla,
    bg: gpui::Hsla,
) -> impl IntoElement {
    div()
        .id("mongo-flatten-hint")
        .w_full()
        .flex_none()
        .px(px(12.0))
        .py(px(5.0))
        .border_b_1()
        .border_color(border)
        .bg(bg)
        .text_xs()
        .text_color(muted)
        .child(SharedString::from(format!(
            "已钻取「{col}」· {n} 条（清空上方过滤列恢复）"
        )))
}

fn empty_hint(text: impl Into<SharedString>, color: gpui::Hsla) -> gpui::Stateful<gpui::Div> {
    div()
        .id("mongo-result-hint")
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .px(px(12.0))
        .py(px(10.0))
        .text_xs()
        .text_color(color)
        .child(text.into())
}

fn error_hint(text: String, color: gpui::Hsla) -> gpui::Stateful<gpui::Div> {
    div()
        .id("mongo-result-error")
        .flex_1()
        .px(px(12.0))
        .py(px(10.0))
        .text_xs()
        .text_color(color)
        .child(SharedString::from(text))
}

/// 过滤列框解析结果
pub(crate) struct ParsedFilter {
    /// 钻取路径（object/array 字段或嵌套路径）→ 钻进去只看其内容（裸字段）
    pub(crate) drill_path: Option<String>,
    /// 列过滤（小写、子串匹配）：分号后投影字段，或标量列名
    pub(crate) filters: Vec<String>,
}

/// 解析过滤列框：`钻取路径 ; 投影字段`，按字段类型自动分派。
/// - 路径指向 object/array → 钻进去只看其字段（裸名、不保留其它列）；标量 → 当列过滤（保留其它列）
/// - 分号后字段 = 钻取层只显示这些列；无分号 = 钻取层全部字段
fn classify_filter(raw: &str, docs: &[Value]) -> ParsedFilter {
    let (head, tail) = raw.split_once(';').unwrap_or((raw, ""));
    let mut drill_path = None;
    let mut filters = Vec::new();
    for tok in head.split(',') {
        let t = tok.trim();
        if t.is_empty() {
            continue;
        }
        if t.contains('.') {
            // 嵌套路径钻取（project.items），保留原大小写供取值
            drill_path = Some(t.to_string());
        } else {
            match field_kind(docs, &t.to_ascii_lowercase()) {
                // object / array → 钻进去看里面
                Some(("object" | "array", real)) => drill_path = Some(real),
                // 标量 / 未知字段 → 当普通列过滤（保留"输入列名过滤"旧行为）
                _ => filters.push(t.to_ascii_lowercase()),
            }
        }
    }
    for f in tail.split(',') {
        let f = f.trim();
        if !f.is_empty() {
            filters.push(f.to_ascii_lowercase());
        }
    }
    ParsedFilter {
        drill_path,
        filters,
    }
}

/// 顶层字段类型（大小写不敏感，取首个有值的文档）；返回 (kind, 原字段名)
fn field_kind(docs: &[Value], name_lower: &str) -> Option<(&'static str, String)> {
    for doc in docs {
        let Value::Object(map) = doc else {
            continue;
        };
        for (k, v) in map {
            if k.to_ascii_lowercase() != name_lower {
                continue;
            }
            match v {
                Value::Null => break, // 此文档该字段为空，看下一个文档
                Value::Array(_) => return Some(("array", k.clone())),
                Value::Object(o) if flatten::extjson_cell(o).is_none() => {
                    return Some(("object", k.clone()));
                }
                _ => return Some(("scalar", k.clone())),
            }
        }
    }
    None
}

/// 按 filters 子串匹配列 path（大小写不敏感）→ 列索引；空 filters 或无命中返回 None（全显示）
fn column_indices_for(table: &FlatTable, filters: &[String]) -> Option<Vec<usize>> {
    if filters.is_empty() {
        return None;
    }
    let indices: Vec<usize> = table
        .columns
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            let lower = c.path.to_ascii_lowercase();
            filters.iter().any(|f| lower.contains(f))
        })
        .map(|(i, _)| i)
        .collect();
    if indices.is_empty() {
        None
    } else {
        Some(indices)
    }
}

/// 行过滤：任意单元格子串包含 query（大小写不敏感）→ 行索引；空 query 返回 None（全显示）
fn row_indices_for(table: &FlatTable, query: &str) -> Option<Vec<usize>> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() {
        return None;
    }
    let indices: Vec<usize> = table
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.iter().any(|c| c.text.to_ascii_lowercase().contains(&q)))
        .map(|(i, _)| i)
        .collect();
    Some(indices)
}

#[cfg(test)]
mod tests {
    use super::flatten::Column;
    use super::{FlatTable, classify_filter, column_indices_for};
    use serde_json::json;

    fn sample() -> Vec<serde_json::Value> {
        vec![json!({
            "_id": "x",
            "appId": "a",
            "geoms": [1, 2],
            "project": {"id": "p", "name": "n", "items": {"id": "i"}}
        })]
    }

    #[test]
    fn object_name_drills() {
        // project 是对象 → 钻取；无投影 → filters 空（看全部字段）
        let p = classify_filter("project", &sample());
        assert_eq!(p.drill_path.as_deref(), Some("project"));
        assert!(p.filters.is_empty());
    }

    #[test]
    fn array_name_drills() {
        // geoms 是数组 → 钻取
        let p = classify_filter("geoms", &sample());
        assert_eq!(p.drill_path.as_deref(), Some("geoms"));
    }

    #[test]
    fn scalar_name_filters() {
        // appId 是标量 → 当列过滤（不钻取，保留旧行为）
        let p = classify_filter("appId", &sample());
        assert!(p.drill_path.is_none());
        assert_eq!(p.filters, vec!["appid".to_string()]);
    }

    #[test]
    fn drill_with_projection() {
        // project ; id, name → 钻进 project，投影裸字段 id / name
        let p = classify_filter("project ; id, name", &sample());
        assert_eq!(p.drill_path.as_deref(), Some("project"));
        assert_eq!(p.filters, vec!["id".to_string(), "name".to_string()]);
    }

    #[test]
    fn nested_path_drills() {
        // project.items ; id → 钻到 project.items，投影 id
        let p = classify_filter("project.items ; id", &sample());
        assert_eq!(p.drill_path.as_deref(), Some("project.items"));
        assert_eq!(p.filters, vec!["id".to_string()]);
    }

    fn table_of(cols: &[&str]) -> FlatTable {
        FlatTable {
            columns: cols
                .iter()
                .map(|c| Column {
                    path: c.to_string(),
                    kind: "text",
                })
                .collect(),
            rows: vec![],
        }
    }

    #[test]
    fn column_filter_substring_and_empty() {
        let t = table_of(&["_id", "consume.cost", "id", "name"]);
        // 空 filters → None（全显示）
        assert!(column_indices_for(&t, &[]).is_none());
        // "name" 命中 name 列
        let idx = column_indices_for(&t, &["name".to_string()]).unwrap();
        assert_eq!(idx, vec![3]);
    }
}
