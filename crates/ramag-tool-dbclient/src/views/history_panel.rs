//! 查询历史面板
//!
//! 显示当前连接的查询历史（按时间倒序）。点击某条 → 填回当前 Tab 编辑器。
//!
//! 触发：QueryPanel 顶部的"历史"按钮，点击后 show_history 切换为 true，
//! QueryPanel 不渲染当前 Tab，改渲染本面板。

use std::sync::Arc;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render,
    SharedString, Styled, Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Sizable as _,
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use ramag_app::ConnectionService;
use ramag_domain::entities::{ConnectionId, QueryRecord, QueryStatus};
use tracing::error;

const HISTORY_LIMIT: usize = 200;

#[derive(Debug, Clone)]
pub enum HistoryEvent {
    /// 用户选了某条历史 → 把 SQL 填回当前 Tab
    Selected(QueryRecord),
}

pub struct HistoryPanel {
    service: Arc<ConnectionService>,
    /// 仅显示当前连接的历史（None 时显示所有）
    connection_id: Option<ConnectionId>,
    records: Vec<QueryRecord>,
    loading: bool,
    error: Option<String>,
    /// 搜索输入框：按 SQL preview 大小写不敏感子串过滤
    filter_input: Entity<InputState>,
}

impl EventEmitter<HistoryEvent> for HistoryPanel {}

impl HistoryPanel {
    pub fn new(
        service: Arc<ConnectionService>,
        connection_id: Option<ConnectionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("搜索历史 SQL...")
        });
        cx.observe(&filter_input, |_, _, cx| cx.notify()).detach();

        Self {
            service,
            connection_id,
            records: Vec::new(),
            loading: false,
            error: None,
            filter_input,
        }
    }

    pub fn set_connection(&mut self, id: Option<ConnectionId>, cx: &mut Context<Self>) {
        if self.connection_id != id {
            self.connection_id = id;
            self.refresh(cx);
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        cx.notify();

        let svc = self.service.clone();
        let conn_id = self.connection_id.clone();
        cx.spawn(async move |this, cx| {
            let result = svc.list_history(conn_id.as_ref(), HISTORY_LIMIT).await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(rs) => this.records = rs,
                    Err(e) => {
                        error!(error = %e, "load history failed");
                        this.error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

}

impl Render for HistoryPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let border = theme.border;
        let secondary_bg = theme.secondary;
        let bg = theme.background;
        let muted_bg = theme.muted;
        let danger = theme.danger;
        let success = theme.success;

        let filter_text = self.filter_input.read(cx).value().trim().to_string();
        let records: Vec<QueryRecord> = if filter_text.is_empty() {
            self.records.clone()
        } else {
            let needle = filter_text.to_lowercase();
            self.records
                .iter()
                .filter(|r| r.sql.to_lowercase().contains(&needle))
                .cloned()
                .collect()
        };
        let total_records = self.records.len();
        let visible_records = records.len();
        let loading = self.loading;
        let err = self.error.clone();

        // 列表行
        let mut list_rows: Vec<AnyElement> = Vec::with_capacity(records.len());
        if loading {
            list_rows.push(
                div()
                    .px_4()
                    .py_4()
                    .text_xs()
                    .text_color(muted_fg)
                    .child("加载中...")
                    .into_any_element(),
            );
        } else if let Some(e) = err {
            list_rows.push(
                div()
                    .px_4()
                    .py_4()
                    .text_xs()
                    .text_color(danger)
                    .child(format!("加载失败：{e}"))
                    .into_any_element(),
            );
        } else if records.is_empty() {
            list_rows.push(
                div()
                    .px_4()
                    .py_8()
                    .text_xs()
                    .text_color(muted_fg)
                    .child("暂无历史，执行一次 SQL 后就会出现在这里")
                    .into_any_element(),
            );
        } else {
            for rec in records.into_iter() {
                let rec_for_click = rec.clone();
                let row_id = SharedString::from(format!("hist-{}", rec.id));
                let preview = rec.sql_preview(160);
                let status_color = match rec.status {
                    QueryStatus::Success => success,
                    QueryStatus::Failed => danger,
                };
                let when = rec.executed_at.with_timezone(&chrono::Local);
                let when_text = when.format("%m-%d %H:%M:%S").to_string();
                let meta_text = match rec.status {
                    QueryStatus::Success => format!(
                        "{} · {} 行 · {} ms",
                        rec.connection_name, rec.rows, rec.elapsed_ms
                    ),
                    QueryStatus::Failed => format!(
                        "{} · 失败：{}",
                        rec.connection_name,
                        rec.error.clone().unwrap_or_default()
                    ),
                };

                list_rows.push(
                    h_flex()
                        .id(row_id)
                        .items_start()
                        .gap_3()
                        .px_4()
                        .py(px(10.0))
                        .border_b_1()
                        .border_color(border)
                        .cursor_pointer()
                        .hover(move |this| this.bg(muted_bg))
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(HistoryEvent::Selected(rec_for_click.clone()));
                        }))
                        // 状态点
                        .child(
                            div()
                                .w(px(8.0))
                                .h(px(8.0))
                                .mt(px(6.0))
                                .rounded_full()
                                .bg(status_color)
                                .flex_none(),
                        )
                        // 文本块
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                // SQL 预览（一行）
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(fg)
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(preview),
                                )
                                // 元数据
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .text_xs()
                                        .text_color(muted_fg)
                                        .child(div().child(when_text))
                                        .child(div().child("·"))
                                        .child(div().child(meta_text)),
                                ),
                        )
                        .into_any_element(),
                );
            }
        }

        v_flex()
            .size_full()
            .bg(bg)
            // 顶部工具条：标题 + 计数 + 操作按钮
            .child(
                h_flex()
                    .w_full()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(border)
                    .bg(secondary_bg)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .child("查询历史"),
                    )
                    .child(
                        div().text_xs().text_color(muted_fg).child(
                            if !filter_text.is_empty() {
                                format!("· 命中 {visible_records} / {total_records}")
                            } else {
                                format!("· {total_records} 条")
                            },
                        ),
                    ),
                // 清空 / 刷新 / 关闭按钮已移除：
                // - 清空 / 刷新：低频，未来可放右键菜单或快捷键，先去掉减负
                // - 关闭：QueryPanel 顶部"历史"按钮再次点击即可 toggle 关闭
            )
            // 搜索栏
            .child(
                h_flex()
                    .w_full()
                    .flex_none()
                    .items_center()
                    .px_3()
                    .py_1()
                    .border_b_1()
                    .border_color(border)
                    .bg(secondary_bg)
                    .child(
                        Input::new(&self.filter_input)
                            .small()
                            .bordered(false)
                            .focus_bordered(false)
                            .cleanable(true),
                    ),
            )
            // 列表区
            .child(
                div()
                    .id("history-list-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_scroll()
                    .child(v_flex().children(list_rows)),
            )
    }
}
