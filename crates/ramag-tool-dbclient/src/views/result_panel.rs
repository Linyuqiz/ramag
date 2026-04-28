//! 结果集面板
//!
//! Stage 3：简单 Table 渲染 QueryResult，Stage 4 升级为虚拟滚动 + 列宽拖拽。
//!
//! 三种渲染状态：
//! - Empty：未执行查询时
//! - Error：执行报错
//! - Result：正常结果（前 N 行）

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;

use gpui::{
    ClickEvent, ClipboardItem, Context, Entity, Focusable as _, IntoElement, ParentElement, Point,
    Render, ScrollHandle, ScrollStrategy, SharedString, Styled, UniformListScrollHandle, Window,
    div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, IconName, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::InputState,
    notification::Notification,
    v_flex,
};
use ramag_app::ConnectionService;
use ramag_app::usecases::export;
use ramag_domain::entities::{Column, ColumnKind, ConnectionConfig, Query, QueryResult, Value};
use tracing::{error, info};

use crate::actions::{
    CopyCellValue, CopySelectedColumn, ExportCsv, ExportJson, ExportMarkdown, FindInResults,
};
use crate::views::result_table::render_table;

/// UI 表格最多渲染行数（超出截断 + 状态栏提示"已截断"）
/// 提到 10000 配合 uniform_list 行级虚拟化：滚动复用屏幕内 ~30 行 cell，
/// 排序 / 行过滤是 O(N) 操作，1w 行约 30-50ms 可接受；超过这个量级建议
/// 走 SQL 端 ORDER BY / WHERE，未来改流式加载再放宽
pub(super) const MAX_ROWS_DISPLAY: usize = 10_000;

/// 结果集状态
#[derive(Debug, Clone, Default)]
pub enum ResultState {
    #[default]
    Empty,
    Running,
    Error(String),
    Ok(QueryResult),
}

/// 排序方向
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

pub struct ResultPanel {
    state: ResultState,
    /// 异步任务（如导出）完成后挂这里，下次 render 在 window 上下文里 push
    pending_notification: Option<Notification>,
    /// 当前选中单元格 (row_idx, col_idx)，用于高亮 + 未来 ⌘C 复制
    selected_cell: Option<(usize, usize)>,
    /// 多选行：表格首列 checkbox 勾选的行索引集合
    /// 与 selected_cell 独立：cell 是单元格高亮，rows 是批量操作（删除等）的目标
    selected_rows: BTreeSet<usize>,
    /// 当前结果对应的源 SQL（QueryTab 在 run/explain 后注入），用于推断
    /// 复制行为 INSERT 时的目标表名
    source_sql: Option<String>,
    /// 上游显式注入的目标 (schema, table)：表树点击时由 QueryPanel 传入
    /// 优先级高于 source_sql 解析：避开"反引号内带短横线被 tokenizer 吞掉"等坑
    pinned_target: Option<(Option<String>, String)>,
    /// 列宽手动覆盖：用户拖动列分隔线后写入；新结果集到来时清空
    /// 索引按 result.columns 顺序，None 表示用 estimate 默认
    col_width_overrides: Vec<Option<gpui::Pixels>>,
    /// 当前排序列与方向：单击列头切换 None→Asc→Desc→None
    sort_by: Option<(usize, SortDir)>,
    /// 列过滤输入框：逗号分隔多列名（命中即显示该列），与行过滤独立叠加
    column_filter_input: Entity<InputState>,
    /// 行过滤输入框：单一关键字，按行内容大小写不敏感子串过滤
    row_filter_input: Entity<InputState>,
    /// 单元格编辑弹框输入框：仅弹框打开期间持有；下次打开时替换释放旧的
    cell_edit_input: Option<Entity<InputState>>,
    /// 行内编辑用的执行器（由 QueryTab 注入）；None 表示仅展示模式不能 UPDATE
    service: Option<Arc<ConnectionService>>,
    connection: Option<ConnectionConfig>,
    /// 新增草稿行：表格末尾追加可编辑空行 + 状态栏「提交 / 取消」
    pending_insert: Option<PendingInsert>,
    /// 结果表格虚拟列表的垂直滚动句柄（uniform_list 用）
    /// 切表 / 重跑后归位首行；新增草稿行后滚到 pending 行让用户看到
    uniform_scroll: UniformListScrollHandle,
    /// 外层水平滚动 div 的 ScrollHandle（双向 ScrollHandle，仅用 X 维度）
    /// 跨 render 保持 X scroll 位置不被 reset；切表时归位左侧
    h_scroll: ScrollHandle,
    /// 列过滤框的补全候选源：每次新结果到达时把列名写入
    /// 共享给 ColumnFilterCompletionProvider，让它读最新列
    column_completion_source: Arc<RwLock<Vec<String>>>,
    /// SHOW WARNINGS 面板是否展开：默认收起，用户点 banner 切换
    /// 新结果集到达时自动重置为 false（避免上次展开状态污染当前结果）
    warnings_expanded: bool,
}

impl ResultPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // 列过滤补全：把列名 Arc 共享给 provider，新结果到达时本 panel 写入即可
        let column_completion_source: Arc<RwLock<Vec<String>>> = Arc::new(RwLock::new(Vec::new()));
        let provider = crate::sql_completion::ColumnFilterCompletionProvider::new_rc(
            column_completion_source.clone(),
        );
        // 单行 Input：cleanable(X 按钮) 仅在 single_line 渲染，必须保持单行
        // 方向键由 query_tab 工具条侧外层 div 拦截 MoveUp/MoveDown action 转发给
        // InputState::handle_action_for_context_menu，让补全菜单选项导航生效
        let column_filter_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("过滤列（逗号分隔多列名）");
            // 装载列名补全 provider：用户键入时弹候选
            state.lsp.completion_provider = Some(provider);
            state
        });
        let row_filter_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("过滤行（任意单元格包含）"));
        // 输入变化 → 触发 ResultPanel 重渲染（filter 应用在 render_table）
        cx.observe(&column_filter_input, |_, _, cx| cx.notify())
            .detach();
        cx.observe(&row_filter_input, |_, _, cx| cx.notify())
            .detach();

        Self {
            state: ResultState::Empty,
            pending_notification: None,
            selected_cell: None,
            source_sql: None,
            col_width_overrides: Vec::new(),
            sort_by: None,
            column_filter_input,
            row_filter_input,
            cell_edit_input: None,
            service: None,
            connection: None,
            pinned_target: None,
            selected_rows: BTreeSet::new(),
            pending_insert: None,
            uniform_scroll: UniformListScrollHandle::new(),
            h_scroll: ScrollHandle::new(),
            column_completion_source,
            warnings_expanded: false,
        }
    }

    /// 暴露给 result_table 用于 .track_scroll
    pub(super) fn uniform_scroll(&self) -> &UniformListScrollHandle {
        &self.uniform_scroll
    }

    /// 外层水平 div 的 ScrollHandle
    pub(super) fn h_scroll(&self) -> &ScrollHandle {
        &self.h_scroll
    }

    /// 进入新增模式：表格末尾追加可编辑草稿行（DataGrip 风格）
    /// 由 QueryTab 工具条「+」按钮异步拉完 columns 后调
    pub fn start_insert(
        &mut self,
        schema: String,
        table: String,
        columns: Vec<Column>,
        inputs: Vec<Entity<InputState>>,
        cx: &mut Context<Self>,
    ) {
        self.pending_insert = Some(PendingInsert {
            schema,
            table,
            columns,
            inputs,
        });
        // 草稿行作为 uniform_list 最后一项；大数据集下默认不在视口
        // 滚到 pending_idx 让用户立刻看到（item_count 在下一帧 render 后才 +1，
        // GPUI 的 scroll_to_item 是 deferred 到 prepaint，那时 list 已含 pending 行）
        let pending_idx = if let ResultState::Ok(qr) = &self.state {
            qr.rows.len().min(MAX_ROWS_DISPLAY)
        } else {
            0
        };
        self.uniform_scroll
            .scroll_to_item(pending_idx, ScrollStrategy::Center);
        cx.notify();
    }

    /// 当前是否在新增草稿状态
    pub(super) fn pending_insert(&self) -> Option<&PendingInsert> {
        self.pending_insert.as_ref()
    }

    /// 退出新增模式（取消按钮调用）
    pub(super) fn cancel_insert(&mut self, cx: &mut Context<Self>) {
        self.pending_insert = None;
        cx.notify();
    }

    /// 提交新增：遍历每列 InputState 校验后调 apply_insert_async
    pub(super) fn submit_insert(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_insert.as_ref() else {
            return;
        };
        let mut values: Vec<(String, Value)> = Vec::new();
        let mut err: Option<String> = None;
        for (col, input) in pending.columns.iter().zip(pending.inputs.iter()) {
            let text = input.read(cx).value().to_string();
            let nullable = col.nullable;
            let has_default = col.default_value.is_some() || col.is_primary_key;
            match parse_value_for_kind(col.data_type.kind, &text, nullable, has_default) {
                Ok(Some(v)) => values.push((col.name.clone(), v)),
                Ok(None) => {} // 跳过让 DB 用 DEFAULT
                Err(msg) => {
                    err = Some(format!("{}: {msg}", col.name));
                    break;
                }
            }
        }
        if let Some(msg) = err {
            self.pending_notification = Some(Notification::error(msg).autohide(true));
            cx.notify();
            return;
        }
        if values.is_empty() {
            self.pending_notification = Some(Notification::warning("请至少填一列").autohide(true));
            cx.notify();
            return;
        }
        self.apply_insert_async(values, cx);
        // 提交后立即退出草稿模式（apply 是异步的，结果通过 toast 反馈）
        self.pending_insert = None;
        cx.notify();
    }

    /// 上游（QueryTab.run）注入精确目标表，避免 SQL parse 误差
    pub fn set_pinned_target(&mut self, target: Option<(Option<String>, String)>) {
        self.pinned_target = target;
    }

    /// 当前结果集对应的目标表的反引号字符串：优先用 pinned_target，再回退 SQL 解析
    fn current_table_ref(&self) -> Option<String> {
        if let Some((schema, table)) = &self.pinned_target {
            let escape = |s: &str| s.replace('`', "``");
            return Some(match schema {
                Some(s) => format!("`{}`.`{}`", escape(s), escape(table)),
                None => format!("`{}`", escape(table)),
            });
        }
        self.source_sql.as_deref().and_then(extract_first_table_ref)
    }

    /// QueryTab 注入执行器：行内编辑（UPDATE 单元格）需要 service + connection
    pub fn set_executor(
        &mut self,
        service: Option<Arc<ConnectionService>>,
        connection: Option<ConnectionConfig>,
    ) {
        self.service = service;
        self.connection = connection;
    }

    /// 单元格编辑弹框：保活 InputState 引用（避免 dialog move 闭包丢失后失活）
    pub(super) fn set_cell_edit_input(&mut self, input: Option<Entity<InputState>>) {
        self.cell_edit_input = input;
    }

    /// 弹框打开前同步取数据：列名 + 当前 cell 文本 + 是否能推断主键
    /// （在 listener 上下文调用，避免 cell_edit_dialog 内 panel.read 二次借用 panic）
    pub(super) fn cell_info(&self, ri: usize, ci: usize) -> Option<(String, String, bool)> {
        let ResultState::Ok(result) = &self.state else {
            return None;
        };
        let col_name = result.columns.get(ci)?.clone();
        let val = result.rows.get(ri)?.values.get(ci)?;
        let has_pk = find_pk_idx(result).is_some();
        // 编辑弹框初值：JSON 列走 pretty 多行（其它类型与 clipboard 字符串一致）
        Some((col_name, val.display_for_edit(), has_pk))
    }

    /// QueryTab 执行成功后调用：把源 SQL 记录下来供 "复制 INSERT" 使用
    pub fn set_source_sql(&mut self, sql: Option<String>) {
        self.source_sql = sql;
    }

    pub fn set_state(&mut self, state: ResultState, cx: &mut Context<Self>) {
        // 同步列名补全源：新结果集到达时刷新候选；其它状态清空
        match &state {
            ResultState::Ok(qr) => {
                *self.column_completion_source.write() = qr.columns.clone();
            }
            _ => {
                self.column_completion_source.write().clear();
            }
        }
        self.state = state;
        // 数据集变更后清除选中、排序、列宽覆盖、新增草稿
        self.selected_cell = None;
        self.selected_rows.clear();
        self.sort_by = None;
        self.col_width_overrides.clear();
        self.pending_insert = None;
        // 新结果到达：warnings 面板默认收起（避免上次展开污染本次）
        self.warnings_expanded = false;
        // 切表/重跑时双向归位：垂直回顶 + 水平回左
        self.uniform_scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.h_scroll.set_offset(Point::new(px(0.0), px(0.0)));
        cx.notify();
    }

    /// 多选：当前已勾选的行索引集合
    pub(super) fn selected_rows(&self) -> &BTreeSet<usize> {
        &self.selected_rows
    }

    /// 多选：toggle 单行勾选状态
    pub(super) fn toggle_row_selected(&mut self, ri: usize, cx: &mut Context<Self>) {
        if !self.selected_rows.remove(&ri) {
            self.selected_rows.insert(ri);
        }
        cx.notify();
    }

    /// 多选：表头全选 / 全不选切换（基于当前可见行 0..total）
    pub(super) fn toggle_all_rows(&mut self, total: usize, cx: &mut Context<Self>) {
        if self.selected_rows.len() == total {
            self.selected_rows.clear();
        } else {
            self.selected_rows = (0..total).collect();
        }
        cx.notify();
    }

    /// 列宽覆盖：拖拽列分隔线时调用
    pub(super) fn set_col_width_override(&mut self, col_ix: usize, width: gpui::Pixels) {
        // 按需扩容
        let n_cols = match &self.state {
            ResultState::Ok(r) => r.columns.len(),
            _ => return,
        };
        if self.col_width_overrides.len() != n_cols {
            self.col_width_overrides.resize(n_cols, None);
        }
        if col_ix < self.col_width_overrides.len() {
            self.col_width_overrides[col_ix] = Some(width);
        }
    }

    pub(super) fn col_width_override(&self, col_ix: usize) -> Option<gpui::Pixels> {
        self.col_width_overrides.get(col_ix).copied().flatten()
    }

    /// 切换列排序：None → Asc → Desc → None
    pub(super) fn toggle_sort(&mut self, col_idx: usize, cx: &mut Context<Self>) {
        self.sort_by = match self.sort_by {
            Some((ci, SortDir::Asc)) if ci == col_idx => Some((col_idx, SortDir::Desc)),
            Some((ci, SortDir::Desc)) if ci == col_idx => None,
            _ => Some((col_idx, SortDir::Asc)),
        };
        // 排序后清除单元格选中（行已重排，原坐标无效）
        self.selected_cell = None;
        cx.notify();
    }

    /// 当前排序状态（供 render_table 读取）
    pub(super) fn sort_by(&self) -> Option<(usize, SortDir)> {
        self.sort_by
    }

    /// 当前选中单元格（供 render_table 读取）
    pub(super) fn selected_cell(&self) -> Option<(usize, usize)> {
        self.selected_cell
    }

    /// 由 render_table 内部 listener 调用：更新选中单元格
    pub(super) fn set_selected_cell(&mut self, cell: Option<(usize, usize)>) {
        self.selected_cell = cell;
    }

    /// 由 render_table 内部 listener 调用：挂起通知，下次 render 推入
    pub(super) fn set_pending_notification(&mut self, n: Option<Notification>) {
        self.pending_notification = n;
    }

    /// 切换表时清空两个过滤框：避免上一张表残留的过滤条件挡新表数据
    /// 必须 window：set_value 内部走 replace_text 依赖 display_map 重建
    pub fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.column_filter_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.row_filter_input
            .update(cx, |s, cx| s.set_value("", window, cx));
    }

    /// 列过滤当前文本（trim；空串=无列过滤）
    pub(super) fn column_filter_text(&self, cx: &gpui::App) -> String {
        self.column_filter_input.read(cx).value().trim().to_string()
    }

    /// 行过滤当前文本（trim；空串=无行过滤）
    pub(super) fn row_filter_text(&self, cx: &gpui::App) -> String {
        self.row_filter_input.read(cx).value().trim().to_string()
    }

    /// 工具条嵌入两个 Input：列过滤 / 行过滤
    pub fn column_filter_entity(&self) -> &Entity<InputState> {
        &self.column_filter_input
    }
    pub fn row_filter_entity(&self) -> &Entity<InputState> {
        &self.row_filter_input
    }

    pub fn state(&self) -> &ResultState {
        &self.state
    }

    /// 导出为 CSV / JSON
    ///
    /// 流程：序列化（同步快）→ 弹系统保存对话框（独立线程）→ 写文件 →
    /// 通过 oneshot 把结果送回主线程 → 设置 pending_notification → 触发 render 推 toast
    ///
    /// 范围：selected_rows 非空 → 仅导出勾选的行；否则导出全部
    pub fn export(&mut self, format: ExportFormat, cx: &mut Context<Self>) {
        let base = match &self.state {
            ResultState::Ok(r) => r,
            _ => {
                self.pending_notification =
                    Some(Notification::warning("无可导出的结果").autohide(true));
                cx.notify();
                return;
            }
        };
        if base.rows.is_empty() {
            self.pending_notification =
                Some(Notification::warning("结果为空，无需导出").autohide(true));
            cx.notify();
            return;
        }

        // 勾选了行 → 仅导出勾选行；否则全部
        // 注意：selected_rows 索引语义与渲染层一致（display_rows 迭代 idx），
        //       未排序/过滤时与 result.rows 物理索引相同
        let (result, scope_label) = if !self.selected_rows.is_empty() {
            let mut filtered = base.clone();
            let selected = self.selected_rows.clone();
            filtered.rows = base
                .rows
                .iter()
                .enumerate()
                .filter(|(i, _)| selected.contains(i))
                .map(|(_, r)| r.clone())
                .collect();
            if filtered.rows.is_empty() {
                self.pending_notification =
                    Some(Notification::warning("勾选的行越界，无内容可导出").autohide(true));
                cx.notify();
                return;
            }
            let n = filtered.rows.len();
            (filtered, format!("选中 {n} 行"))
        } else {
            (base.clone(), format!("全部 {} 行", base.rows.len()))
        };

        // 数据序列化（主线程毫秒级）
        let (content, default_name, ext) = match format {
            ExportFormat::Csv => (
                export::to_csv(&result),
                format!(
                    "ramag-export-{}.csv",
                    chrono::Local::now().format("%Y%m%d-%H%M%S")
                ),
                "csv",
            ),
            ExportFormat::Json => (
                export::to_json(&result),
                format!(
                    "ramag-export-{}.json",
                    chrono::Local::now().format("%Y%m%d-%H%M%S")
                ),
                "json",
            ),
            ExportFormat::Markdown => (
                export::to_markdown(&result),
                format!(
                    "ramag-export-{}.md",
                    chrono::Local::now().format("%Y%m%d-%H%M%S")
                ),
                "md",
            ),
        };

        // 异步：用 std::thread + futures::oneshot 把结果送回主线程（不依赖 tokio）
        let (tx, rx) = futures::channel::oneshot::channel::<ExportOutcome>();
        std::thread::spawn(move || {
            let path = rfd::FileDialog::new()
                .set_file_name(&default_name)
                .add_filter(ext, &[ext])
                .save_file();
            let outcome = match path {
                None => ExportOutcome::Cancelled,
                Some(p) => match std::fs::write(&p, content) {
                    Ok(_) => ExportOutcome::Saved(p),
                    Err(e) => ExportOutcome::Failed(e.to_string()),
                },
            };
            let _ = tx.send(outcome);
        });

        cx.spawn(async move |this, cx| {
            let outcome = rx.await.unwrap_or(ExportOutcome::Cancelled);
            let _ = this.update(cx, |this, cx| {
                let n = match outcome {
                    ExportOutcome::Saved(p) => {
                        info!(path = %p.display(), scope = %scope_label, "exported");
                        // 仅显示文件名（不含路径），避免 toast 过宽
                        let file_name = p
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "导出完成".to_string());
                        // 标题带导出范围（全部 N 行 / 选中 K 行）让用户看清
                        Notification::success(file_name)
                            .title(format!("导出成功 · {scope_label}"))
                            .autohide(true)
                    }
                    ExportOutcome::Cancelled => Notification::info("已取消导出").autohide(true),
                    ExportOutcome::Failed(msg) => {
                        error!(error = %msg, "export failed");
                        // 错误消息也截断到合理长度
                        let short = if msg.chars().count() > 80 {
                            let truncated: String = msg.chars().take(80).collect();
                            format!("{truncated}…")
                        } else {
                            msg
                        };
                        Notification::error(short).title("导出失败").autohide(true)
                    }
                };
                this.pending_notification = Some(n);
                cx.notify();
            });
        })
        .detach();
    }
}

enum ExportOutcome {
    Saved(PathBuf),
    Cancelled,
    Failed(String),
}

#[derive(Clone, Copy)]
pub enum ExportFormat {
    Csv,
    Json,
    Markdown,
}

impl Render for ResultPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 把异步任务挂的通知 push 到全局 toast
        if let Some(n) = self.pending_notification.take() {
            window.push_notification(n, cx);
        }

        // 监听 ExportCsv / ExportJson Action（dropdown menu 触发）
        // 这里通过 cx.on_action 注册，下方 v_flex 会接收
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let border = theme.border;
        let secondary_bg = theme.secondary;
        let danger = theme.danger;
        let muted_bg = theme.muted;
        let accent = theme.accent;

        let content = match &self.state {
            ResultState::Empty => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_1()
                .text_color(muted_fg)
                .text_xs()
                .child("点左侧表名查看数据")
                .child("或按 ⌘E 唤出 SQL 编辑器，再按 ⌘↵ 运行")
                .into_any_element(),

            ResultState::Running => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(muted_fg)
                .text_xs()
                .child("执行中...")
                .into_any_element(),

            ResultState::Error(msg) => {
                let msg_for_copy = msg.clone();
                v_flex()
                    .size_full()
                    .p_4()
                    .gap_2()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(danger)
                                    .child("执行失败"),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("copy-error")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Copy)
                                    .tooltip("复制错误信息")
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            msg_for_copy.clone(),
                                        ));
                                        this.pending_notification = Some(
                                            Notification::success("已复制错误信息").autohide(true),
                                        );
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(div().text_xs().text_color(fg).child(msg.clone()))
                    .into_any_element()
            }

            ResultState::Ok(result) => render_table(
                self,
                result.clone(),
                fg,
                muted_fg,
                secondary_bg,
                border,
                muted_bg,
                accent,
                cx,
            )
            .into_any_element(),
        };

        // SHOW WARNINGS banner：仅 Ok 且有 warnings 时渲染；
        // 永远在结果区上方，让用户无法忽略
        let warnings_banner = self.render_warnings_banner(cx);

        // 包一层 div 以挂 on_action：dropdown_menu 触发的 ExportCsv/ExportJson
        // 由这里捕获后调用 self.export
        let mut root = v_flex()
            .size_full()
            .min_w_0()
            .on_action(cx.listener(|this, _: &ExportCsv, _, cx| {
                this.export(ExportFormat::Csv, cx);
            }))
            .on_action(cx.listener(|this, _: &ExportJson, _, cx| {
                this.export(ExportFormat::Json, cx);
            }))
            // ⌘F：聚焦行过滤输入框（行过滤更常用；列过滤需手动点击）
            .on_action(cx.listener(|this, _: &FindInResults, window, cx| {
                let handle = this.row_filter_input.read(cx).focus_handle(cx);
                handle.focus(window, cx);
                cx.notify();
            }))
            // 右键菜单 actions：基于 selected_cell 取数据
            .on_action(cx.listener(|this, _: &CopyCellValue, _, cx| {
                this.copy_selected_cell(cx);
            }))
            .on_action(cx.listener(|this, _: &CopySelectedColumn, _, cx| {
                this.copy_selected_column_name(cx);
            }))
            .on_action(cx.listener(|this, _: &ExportMarkdown, _, cx| {
                this.export(ExportFormat::Markdown, cx);
            }));
        if let Some(banner) = warnings_banner {
            root = root.child(banner);
        }
        // 结果区放在 v_flex 内并占满剩余空间（min_h_0 让 uniform_list 内部能滚动）
        root.child(div().flex_1().min_h_0().child(content))
    }
}

impl ResultPanel {
    /// 渲染 SHOW WARNINGS 提示条
    ///
    /// - 当且仅当 state == Ok 且 warnings 非空时返回 Some
    /// - 收起态：显示 "⚠ N 条服务端警告（点击展开）"
    /// - 展开态：列出每条 [Level Code] message；最多 20 条，超出加 "更多 K 条"
    fn render_warnings_banner(&self, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        let ResultState::Ok(qr) = &self.state else {
            return None;
        };
        if qr.warnings.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let warning_color = theme.warning;
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let border = theme.border;
        let secondary_bg = theme.secondary;

        let count = qr.warnings.len();
        let expanded = self.warnings_expanded;
        let header_label = if expanded {
            format!("⚠ {count} 条服务端警告（点击收起）")
        } else {
            format!("⚠ {count} 条服务端警告（点击展开）")
        };
        // 标题行：可点击切换展开
        let header = h_flex()
            .id(SharedString::from("warnings-header"))
            .w_full()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .cursor_pointer()
            .bg(secondary_bg)
            .border_b_1()
            .border_color(border)
            .child(
                div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(warning_color)
                    .child(header_label),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.warnings_expanded = !this.warnings_expanded;
                cx.notify();
            }));

        if !expanded {
            return Some(header.into_any_element());
        }

        // 展开态：每行 [Level Code] message；最多 20 条，避免极端情况撑爆 UI
        const MAX_VISIBLE: usize = 20;
        let mut rows: Vec<gpui::AnyElement> =
            Vec::with_capacity(qr.warnings.len().min(MAX_VISIBLE) + 1);
        for w in qr.warnings.iter().take(MAX_VISIBLE) {
            let line = format!("[{} {}] {}", w.level, w.code, w.message);
            rows.push(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(fg)
                    .child(line)
                    .into_any_element(),
            );
        }
        if count > MAX_VISIBLE {
            rows.push(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(muted_fg)
                    .child(format!("…更多 {} 条", count - MAX_VISIBLE))
                    .into_any_element(),
            );
        }

        Some(
            v_flex()
                .w_full()
                .flex_none()
                .border_b_1()
                .border_color(border)
                .child(header)
                .child(v_flex().py_1().children(rows))
                .into_any_element(),
        )
    }

    /// 复制选中单元格完整值
    fn copy_selected_cell(&mut self, cx: &mut Context<Self>) {
        let Some((ri, ci)) = self.selected_cell else {
            return;
        };
        let ResultState::Ok(result) = &self.state else {
            return;
        };
        let Some(val) = result.rows.get(ri).and_then(|r| r.values.get(ci)) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(val.to_clipboard_string()));
        self.pending_notification = Some(Notification::success("已复制单元格").autohide(true));
        cx.notify();
    }

    /// 复制选中列的列名
    fn copy_selected_column_name(&mut self, cx: &mut Context<Self>) {
        let Some((_, ci)) = self.selected_cell else {
            return;
        };
        let ResultState::Ok(result) = &self.state else {
            return;
        };
        let Some(name) = result.columns.get(ci).cloned() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(name.clone()));
        self.pending_notification =
            Some(Notification::success(format!("已复制列名 {name}")).autohide(true));
        cx.notify();
    }

    /// 删除前的预览数据：(row_idx, "列=值" 简短文案)；调用方拿去给 confirm dialog 用
    /// 优先用主键列做预览，没主键用第一列
    pub(super) fn delete_preview(&self) -> Option<(usize, String)> {
        let (ri, _) = self.selected_cell?;
        let ResultState::Ok(result) = &self.state else {
            return None;
        };
        let row = result.rows.get(ri)?;
        let idx = find_pk_idx(result).unwrap_or(0);
        let col = result.columns.get(idx)?.clone();
        let val = row
            .values
            .get(idx)
            .map(|v| v.display_preview(60))
            .unwrap_or_default();
        Some((ri, format!("{col} = {val}")))
    }

    /// 批量删除前的预览：返回 (排序去重后的 indices, "N 行预览" 文案)
    /// 文案最多展示前 3 行的「列=值」，多余用 "...还有 N 行"
    pub(super) fn delete_preview_multi(&self) -> Option<(Vec<usize>, String)> {
        if self.selected_rows.is_empty() {
            return None;
        }
        let ResultState::Ok(result) = &self.state else {
            return None;
        };
        let mut indices: Vec<usize> = self
            .selected_rows
            .iter()
            .copied()
            .filter(|i| *i < result.rows.len())
            .collect();
        indices.sort();
        indices.dedup();
        if indices.is_empty() {
            return None;
        }
        let pk_or_first = find_pk_idx(result).unwrap_or(0);
        let preview_col = result.columns.get(pk_or_first).cloned().unwrap_or_default();
        let mut samples: Vec<String> = indices
            .iter()
            .take(3)
            .filter_map(|&ri| {
                let row = result.rows.get(ri)?;
                let val = row
                    .values
                    .get(pk_or_first)
                    .map(|v| v.display_preview(40))
                    .unwrap_or_default();
                Some(format!("{preview_col} = {val}"))
            })
            .collect();
        if indices.len() > 3 {
            samples.push(format!("…还有 {} 行", indices.len() - 3));
        }
        let summary = format!("将删除 {} 行：{}", indices.len(), samples.join(" / "));
        Some((indices, summary))
    }

    /// 批量执行 DELETE：每行独立 SQL（DELETE ... WHERE ... LIMIT 1），串行 await
    /// 选这种"循环单删"而非 IN 子句：无主键时也能稳定走（依赖 build_pk_where 兜底逻辑）
    pub(super) fn execute_delete_rows_async(
        &mut self,
        indices: Vec<usize>,
        cx: &mut Context<Self>,
    ) {
        let Some(svc) = self.service.clone() else {
            self.pending_notification =
                Some(Notification::warning("当前未注入连接，无法删除").autohide(true));
            cx.notify();
            return;
        };
        let Some(conn) = self.connection.clone() else {
            self.pending_notification =
                Some(Notification::warning("当前未注入连接，无法删除").autohide(true));
            cx.notify();
            return;
        };
        let ResultState::Ok(result) = &self.state else {
            return;
        };
        let table_ref = match self.current_table_ref() {
            Some(t) => t,
            None => {
                self.pending_notification = Some(
                    Notification::error("无法识别目标表，请先用 SELECT 单表查询后再删除")
                        .autohide(true),
                );
                cx.notify();
                return;
            }
        };

        let by_pk = find_pk_idx(result).is_some();
        let strategy = if by_pk {
            "按主键"
        } else {
            "按全列等值"
        };

        // 主线程一次性把每行的 SQL 算好，避免 spawn 闭包内借 result
        let plans: Vec<(usize, String)> = indices
            .iter()
            .filter_map(|&ri| {
                let row = result.rows.get(ri)?;
                let where_clause = build_pk_where(result, row);
                let sql = format!("DELETE FROM {table_ref} WHERE {where_clause} LIMIT 1;");
                Some((ri, sql))
            })
            .collect();
        if plans.is_empty() {
            return;
        }

        cx.spawn(async move |this, cx| {
            let mut deleted: Vec<usize> = Vec::new();
            let mut last_err: Option<String> = None;
            for (ri, sql) in plans {
                let q = Query::new(sql);
                match svc.execute_with_history(&conn, &q).await {
                    Ok(qr) if qr.affected_rows > 0 => deleted.push(ri),
                    Ok(_) => {} // affected_rows=0 跳过（PK 已被改动等）
                    Err(e) => {
                        error!(error = %e, "delete row failed (in batch)");
                        last_err = Some(e.to_string());
                        break;
                    }
                }
            }
            let _ = this.update(cx, |this, cx| {
                // 本地倒序移除已删行：升序移除会导致后续索引偏移
                if let ResultState::Ok(r) = &mut this.state {
                    let mut to_remove = deleted.clone();
                    to_remove.sort_by(|a, b| b.cmp(a));
                    for ri in to_remove {
                        if ri < r.rows.len() {
                            r.rows.remove(ri);
                        }
                    }
                }
                this.selected_rows.clear();
                this.selected_cell = None;
                this.pending_notification = Some(if let Some(e) = last_err {
                    Notification::error(format!("已删除 {} 行后出错：{e}", deleted.len()))
                        .autohide(true)
                } else {
                    Notification::success(format!("已删除 {} 行（{strategy}匹配）", deleted.len()))
                        .autohide(true)
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// 新增行弹框确认后调用：异步执行 INSERT，成功后本地 rows.push
    /// values: 用户输入并已经过校验/转换的 (列名, Value) 对；空 Vec 跳过执行
    /// 列出现在 values 里的才会 INSERT，缺失的列让 DB 用 DEFAULT（适配自增 PK）
    pub(super) fn apply_insert_async(
        &mut self,
        values: Vec<(String, Value)>,
        cx: &mut Context<Self>,
    ) {
        if values.is_empty() {
            return;
        }
        let Some(svc) = self.service.clone() else {
            self.pending_notification =
                Some(Notification::warning("当前未注入连接，无法新增").autohide(true));
            cx.notify();
            return;
        };
        let Some(conn) = self.connection.clone() else {
            self.pending_notification =
                Some(Notification::warning("当前未注入连接，无法新增").autohide(true));
            cx.notify();
            return;
        };
        let table_ref = match self.current_table_ref() {
            Some(t) => t,
            None => {
                self.pending_notification = Some(
                    Notification::error("无法识别目标表，请先用 SELECT 单表查询后再新增")
                        .autohide(true),
                );
                cx.notify();
                return;
            }
        };

        let cols_sql = values
            .iter()
            .map(|(c, _)| format!("`{}`", c.replace('`', "``")))
            .collect::<Vec<_>>()
            .join(", ");
        let vals_sql = values
            .iter()
            .map(|(_, v)| v.to_sql_literal())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("INSERT INTO {table_ref} ({cols_sql}) VALUES ({vals_sql});");
        let q = Query::new(sql);
        // 提前算好 push 用的新行：列顺序按当前结果集列顺序，缺失列填 Null
        let new_row_values: Option<Vec<Value>> = match &self.state {
            ResultState::Ok(r) => Some(
                r.columns
                    .iter()
                    .map(|c| {
                        values
                            .iter()
                            .find(|(name, _)| name.eq_ignore_ascii_case(c))
                            .map(|(_, v)| v.clone())
                            .unwrap_or(Value::Null)
                    })
                    .collect(),
            ),
            _ => None,
        };

        cx.spawn(async move |this, cx| {
            let outcome = svc.execute_with_history(&conn, &q).await;
            let _ = this.update(cx, |this, cx| {
                match outcome {
                    Ok(qr) => {
                        if qr.affected_rows == 0 {
                            this.pending_notification = Some(
                                Notification::warning("INSERT 未影响任何行（请检查约束）")
                                    .autohide(true),
                            );
                        } else {
                            // 本地追加该行（注意：自增列没回填，UI 显示为 NULL；下次刷新会拿到真值）
                            if let (ResultState::Ok(r), Some(vs)) =
                                (&mut this.state, new_row_values)
                            {
                                r.rows.push(ramag_domain::entities::Row { values: vs });
                            }
                            this.pending_notification = Some(
                                Notification::success(format!("已新增 {} 行", qr.affected_rows))
                                    .autohide(true),
                            );
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "insert row failed");
                        this.pending_notification =
                            Some(Notification::error(format!("新增失败：{e}")).autohide(true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 二次确认后真执行 DELETE：异步发到 DB，成功后本地移除该行
    pub(super) fn execute_delete_row_async(&mut self, ri: usize, cx: &mut Context<Self>) {
        let Some(svc) = self.service.clone() else {
            self.pending_notification =
                Some(Notification::warning("当前未注入连接，无法删除").autohide(true));
            cx.notify();
            return;
        };
        let Some(conn) = self.connection.clone() else {
            self.pending_notification =
                Some(Notification::warning("当前未注入连接，无法删除").autohide(true));
            cx.notify();
            return;
        };
        let ResultState::Ok(result) = &self.state else {
            return;
        };
        let Some(row) = result.rows.get(ri).cloned() else {
            return;
        };

        let table_ref = match self.current_table_ref() {
            Some(t) => t,
            None => {
                self.pending_notification = Some(
                    Notification::error("无法识别目标表，请先用 SELECT 单表查询后再删除")
                        .autohide(true),
                );
                cx.notify();
                return;
            }
        };

        let by_pk = find_pk_idx(result).is_some();
        let strategy = if by_pk {
            "按主键"
        } else {
            "按全列等值"
        };
        let where_clause = build_pk_where(result, &row);
        let sql = format!("DELETE FROM {table_ref} WHERE {where_clause} LIMIT 1;");
        let q = Query::new(sql);

        cx.spawn(async move |this, cx| {
            let outcome = svc.execute_with_history(&conn, &q).await;
            let _ = this.update(cx, |this, cx| {
                match outcome {
                    Ok(qr) => {
                        if qr.affected_rows == 0 {
                            this.pending_notification = Some(
                                Notification::warning(
                                    "DELETE 未匹配到记录（请检查主键或行已被改动）",
                                )
                                .autohide(true),
                            );
                        } else {
                            // 本地同步移除该行 + 清掉选中（避免 OOB）
                            if let ResultState::Ok(r) = &mut this.state {
                                if ri < r.rows.len() {
                                    r.rows.remove(ri);
                                }
                            }
                            this.selected_cell = None;
                            this.pending_notification = Some(
                                Notification::success(format!(
                                    "已删除 {} 行（{strategy}匹配）",
                                    qr.affected_rows
                                ))
                                .autohide(true),
                            );
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "delete row failed");
                        this.pending_notification =
                            Some(Notification::error(format!("删除失败：{e}")).autohide(true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 单元格编辑弹框「确认修改」：异步执行 UPDATE，成功后同步本地 cell
    /// 失败 / affected_rows=0 通过 toast 反馈，不直接失败弹框
    pub(super) fn apply_cell_update_async(
        &mut self,
        ri: usize,
        ci: usize,
        new_text: String,
        cx: &mut Context<Self>,
    ) {
        let Some(svc) = self.service.clone() else {
            self.pending_notification =
                Some(Notification::warning("当前未注入连接，无法修改").autohide(true));
            cx.notify();
            return;
        };
        let Some(conn) = self.connection.clone() else {
            self.pending_notification =
                Some(Notification::warning("当前未注入连接，无法修改").autohide(true));
            cx.notify();
            return;
        };
        let ResultState::Ok(result) = &self.state else {
            return;
        };
        let Some(row) = result.rows.get(ri).cloned() else {
            return;
        };
        let Some(col_name) = result.columns.get(ci).cloned() else {
            return;
        };
        let Some(cell_val) = row.values.get(ci).cloned() else {
            return;
        };

        // 不允许在没识别到目标表时执行 UPDATE：避免把 `<table>` 占位 SQL 真发到 DB
        let table_ref = match self.current_table_ref() {
            Some(t) => t,
            None => {
                self.pending_notification = Some(
                    Notification::error("无法识别目标表，请先用 SELECT 单表查询后再编辑")
                        .autohide(true),
                );
                cx.notify();
                return;
            }
        };

        // 主键定位 vs 全列等值兜底：策略名用于 toast 文案
        let by_pk = find_pk_idx(result).is_some();
        let strategy = if by_pk {
            "按主键"
        } else {
            "按全列等值"
        };

        let where_clause = build_pk_where(result, &row);
        let new_literal = escape_new_value_for_old(&cell_val, &new_text);
        let sql = format!(
            "UPDATE {table_ref} SET `{}` = {new_literal} WHERE {where_clause} LIMIT 1;",
            col_name.replace('`', "``"),
        );
        let new_cell_val = build_new_value_for_old(&cell_val, &new_text);
        let q = Query::new(sql);

        cx.spawn(async move |this, cx| {
            let outcome = svc.execute_with_history(&conn, &q).await;
            let _ = this.update(cx, |this, cx| {
                match outcome {
                    Ok(qr) => {
                        if qr.affected_rows == 0 {
                            this.pending_notification = Some(
                                Notification::warning("UPDATE 未匹配到记录（请检查主键）")
                                    .autohide(true),
                            );
                        } else {
                            // 本地同步该 cell：避免重新拉数据
                            if let ResultState::Ok(r) = &mut this.state {
                                if let Some(row) = r.rows.get_mut(ri) {
                                    if let Some(slot) = row.values.get_mut(ci) {
                                        *slot = new_cell_val;
                                    }
                                }
                            }
                            this.pending_notification = Some(
                                Notification::success(format!(
                                    "已更新 {} 行（{strategy}匹配）",
                                    qr.affected_rows
                                ))
                                .autohide(true),
                            );
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "apply cell update failed");
                        this.pending_notification =
                            Some(Notification::error(format!("更新失败：{e}")).autohide(true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// 新增草稿行的状态：渲染层从 ResultPanel.pending_insert 读
pub(super) struct PendingInsert {
    pub schema: String,
    pub table: String,
    pub columns: Vec<Column>,
    pub inputs: Vec<Entity<InputState>>,
}

/// 让 schema/table 字段不报 dead_code（实际我们目前用列展示足够，schema/table 留作未来扩展用）
#[allow(dead_code)]
impl PendingInsert {
    pub fn schema(&self) -> &str {
        &self.schema
    }
    pub fn table(&self) -> &str {
        &self.table
    }
}

/// 把用户输入按列类型转换 Value（提交新增 / 单元格编辑共用）
/// - Ok(Some(v))：填了具体值
/// - Ok(None)：留空 + 有 default → 跳过让 DB 用 DEFAULT
/// - Err(msg)：必填空 / 类型不匹配 / 不可为 NULL 等
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
/// generate_delete / generate_update / apply_cell_update_async 都依赖它
fn find_pk_idx(result: &QueryResult) -> Option<usize> {
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

/// 构造按主键的 WHERE 子句：复用于 generate_delete / generate_update
/// 找到 PK 列就 `pk = val`，否则回退所有列等值（脆弱但安全，由用户审查）
fn build_pk_where(result: &QueryResult, row: &ramag_domain::entities::Row) -> String {
    if let Some(idx) = find_pk_idx(result) {
        let col = result.columns.get(idx).cloned().unwrap_or_default();
        let val = row
            .values
            .get(idx)
            .map(|v| v.to_sql_literal())
            .unwrap_or_else(|| "NULL".into());
        format!("`{}` = {}", col.replace('`', "``"), val)
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
                format!("`{}` = {}", c.replace('`', "``"), v)
            })
            .collect::<Vec<_>>()
            .join(" AND ")
    }
}

/// 按原 cell 类型把用户输入转换成新的 Value（同时供本地刷新 + SQL 字面量）
/// - Null 旧值：空 / "NULL" → Null；其他 → Text
/// - Int / Float：解析成功用对应数值；失败回退 Text
/// - Bool：1/true ↔ true，0/false ↔ false；其他 → Text
/// - 其他类型：直接 Text
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

/// SQL 字面量包装（apply_cell_update_async 用）：build → to_sql_literal
fn escape_new_value_for_old(old: &Value, new_text: &str) -> String {
    build_new_value_for_old(old, new_text).to_sql_literal()
}

/// 从 SQL 提取第一个表引用（带反引号格式化好），用于复制 INSERT 时的目标表
/// schema.table → `s`.`t`；纯 table → `t`
fn extract_first_table_ref(sql: &str) -> Option<String> {
    use crate::sql_completion::extract_tables_in_use_for_prefetch;
    let tables = extract_tables_in_use_for_prefetch(sql);
    let (maybe_schema, table) = tables.into_iter().next()?;
    let escape = |s: &str| s.replace('`', "``");
    let table_q = format!("`{}`", escape(&table));
    Some(match maybe_schema {
        Some(s) => format!("`{}`.{}", escape(&s), table_q),
        None => table_q,
    })
}
