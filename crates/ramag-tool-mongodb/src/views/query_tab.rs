//! 单 Tab 编辑器：JSON 命令编辑器 + 工具条 + 结果区。
//!
//! 编辑器内容是 MongoDB 原生 runCommand 风格的 JSON：
//!   `{"find": "users", "filter": {...}, "limit": 10000}` / `{"aggregate": "...", "pipeline": [...], "cursor": {}}` / `{"count": "users", "query": {...}}`
//! 运行后若返回带 `cursor.firstBatch`，自动展开为文档列表；否则把整个返回当单文档展示

use std::sync::Arc;
use std::time::Instant;

use gpui::{
    Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window, div,
    prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use ramag_app::MongoService;
use ramag_domain::entities::{ConnectionConfig, MongoQueryResult};
use serde_json::Value;
use tracing::{info, warn};

use crate::actions::{FormatMongoJson, RunMongoQuery};
use crate::views::result_panel::ResultPanel;

pub struct MongoQueryTab {
    pub(crate) service: Arc<MongoService>,
    pub(crate) config: ConnectionConfig,
    /// 当前默认 db；由树或连接配置同步
    pub(crate) database: String,
    /// 当前 collection（仅用于 prefill 时标记）
    pub(crate) collection: Option<String>,
    /// JSON 命令编辑器（多行）
    pub(crate) editor: Entity<InputState>,
    /// 编辑器显隐（默认 false 隐藏，与 dbclient 一致；cmd-e 切换）
    pub(crate) show_editor: bool,
    /// 结果展示
    pub(crate) result: Entity<ResultPanel>,
    pub(crate) running: bool,
    _subscriptions: Vec<Subscription>,
}

impl MongoQueryTab {
    pub fn new(
        service: Arc<MongoService>,
        config: ConnectionConfig,
        default_db: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let database = default_db
            .or_else(|| config.database.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "admin".to_string());

        // multi_line 默认占满，line 高度由父级控制
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("{\"find\": \"users\", \"filter\": {}, \"limit\": 10000}")
                .multi_line(true)
                .default_value(default_command_template())
        });
        let result = cx.new(|cx_inner| ResultPanel::new(window, cx_inner));

        Self {
            service,
            config,
            database,
            collection: None,
            editor,
            show_editor: false,
            result,
            running: false,
            _subscriptions: Vec::new(),
        }
    }

    /// 切换编辑器显隐，返回当前可见状态
    pub fn toggle_editor(&mut self, cx: &mut Context<Self>) -> bool {
        self.show_editor = !self.show_editor;
        cx.notify();
        self.show_editor
    }

    /// 由 QueryPanel 同步全局开关给新建 / 切换的 Tab
    pub fn set_show_editor(&mut self, v: bool, cx: &mut Context<Self>) {
        if self.show_editor != v {
            self.show_editor = v;
            cx.notify();
        }
    }

    /// 用 collection 名预填一段 `find` 模板；由树点击 collection 时调
    pub fn prefill_for_collection(
        &mut self,
        database: String,
        collection: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.database = database;
        self.collection = Some(collection.clone());
        let cmd = format!(
            "{{\n  \"find\": \"{}\",\n  \"filter\": {{}},\n  \"limit\": 10000\n}}",
            collection
        );
        self.editor.update(cx, |s, cx| {
            s.set_value(cmd, window, cx);
        });
        cx.notify();
    }

    /// 设置当前 db（点击树上 db 行时调）
    pub fn set_database(&mut self, db: String, cx: &mut Context<Self>) {
        if self.database != db {
            self.database = db;
            cx.notify();
        }
    }

    pub fn title(&self) -> String {
        if let Some(coll) = &self.collection {
            format!("{}.{coll}", self.database)
        } else {
            self.database.clone()
        }
    }

    /// 运行：编辑器内容解析为 JSON 命令 → run_command → 智能解包 cursor.firstBatch
    pub fn run(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        let text = self.editor.read(cx).value().to_string();
        let cmd: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                self.result.update(cx, |p, cx| {
                    p.set_error(format!("JSON 解析失败：{e}"), cx);
                });
                return;
            }
        };
        if !cmd.is_object() {
            self.result.update(cx, |p, cx| {
                p.set_error("顶层 JSON 必须是对象".to_string(), cx);
            });
            return;
        }

        let svc = self.service.clone();
        let conf = self.config.clone();
        let db = self.database.clone();
        let cmd_text = text.clone();
        self.running = true;
        self.result.update(cx, |p, cx| p.set_running(cx));
        let result_handle = self.result.clone();

        cx.spawn(async move |this, cx| {
            let start = Instant::now();
            let outcome = svc.run_command(&conf, &db, cmd).await;
            let elapsed_ms = start.elapsed().as_millis() as u64;
            let qr: ramag_domain::error::Result<MongoQueryResult> = match outcome {
                Ok(resp) => Ok(parse_run_command_response(resp, elapsed_ms)),
                Err(e) => Err(e),
            };
            // 写历史在同 task 顺序执行，避免 DomainError 不实现 Clone 的借用难题
            svc.append_history(&conf, cmd_text, &qr).await;

            let _ = this.update(cx, |this, cx| {
                this.running = false;
                match qr {
                    Ok(r) => {
                        info!(
                            db = %this.database,
                            docs = r.documents.len(),
                            ms = r.elapsed_ms,
                            "mongo command done"
                        );
                        result_handle.update(cx, |p, cx| p.set_result(r, cx));
                    }
                    Err(e) => {
                        warn!(error = %e, "mongo command failed");
                        result_handle.update(cx, |p, cx| p.set_error(e.to_string(), cx));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 聚焦编辑器（新建 / 切换 / 关闭 Tab 后由 QueryPanel 调用，避免用户再点一下）
    pub fn focus_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |state, cx| {
            state.focus(window, cx);
        });
    }

    /// 格式化编辑器 JSON
    pub fn format_json(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).value().to_string();
        let parsed: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                self.result.update(cx, |p, cx| {
                    p.set_error(format!("格式化失败（JSON 无效）：{e}"), cx);
                });
                return;
            }
        };
        if let Ok(pretty) = serde_json::to_string_pretty(&parsed) {
            self.editor.update(cx, |s, cx| {
                s.set_value(pretty, window, cx);
            });
            cx.notify();
        }
    }
}

impl Render for MongoQueryTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bg = cx.theme().background;
        let fg = cx.theme().foreground;
        let border = cx.theme().border;
        let _ = self.title(); // 当前 db.collection 标签已由左侧 collection_tree header 展示，不再重复

        // 工具条 + 编辑器：仅在 show_editor=true 时整体显示；
        // 工具条不再含「当前：xxx」标签（左侧 collection_tree 顶部已展示），只保留 [运行] [格式化]
        let running = self.running;
        let show_editor = self.show_editor;
        let editor_clone = self.editor.clone();

        v_flex()
            .size_full()
            .bg(bg)
            .text_color(fg)
            .key_context("MongoQueryTab")
            .on_action(cx.listener(|this, _: &RunMongoQuery, window, cx| this.run(window, cx)))
            .on_action(
                cx.listener(|this, _: &FormatMongoJson, window, cx| this.format_json(window, cx)),
            )
            .when(show_editor, move |v| {
                v.child(
                    h_flex()
                        .px(px(8.0))
                        .py(px(6.0))
                        .gap(px(6.0))
                        .items_center()
                        .border_b_1()
                        .border_color(border)
                        .child(div().flex_1())
                        .child(
                            Button::new("mongo-run")
                                .primary()
                                .xsmall()
                                .icon(IconName::Play)
                                .label("运行")
                                .disabled(running)
                                .on_click(cx.listener(|this, _, window, cx| this.run(window, cx))),
                        )
                        .child(
                            Button::new("mongo-format")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Settings)
                                .label("格式化")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.format_json(window, cx)
                                })),
                        ),
                )
                .child(
                    div()
                        .h(px(220.0))
                        .border_b_1()
                        .border_color(border)
                        .px(px(8.0))
                        .py(px(6.0))
                        .child(Input::new(&editor_clone).small().h_full()),
                )
            })
            .child(div().flex_1().min_h_0().child(self.result.clone()))
    }
}

/// 默认编辑器模板（无 collection 时显示）
fn default_command_template() -> String {
    "{\n  \"ping\": 1\n}".to_string()
}

/// 解析 run_command 返回：智能识别 cursor.firstBatch / 普通文档 / 错误结构
fn parse_run_command_response(response: Value, elapsed_ms: u64) -> MongoQueryResult {
    // 优先：cursor.firstBatch（find / aggregate / listCollections / listIndexes）
    if let Some(batch) = response
        .get("cursor")
        .and_then(|c| c.get("firstBatch"))
        .and_then(|b| b.as_array())
        .cloned()
    {
        return MongoQueryResult::read(batch, elapsed_ms);
    }
    // 次优：count 返回 `n`
    if let Some(n) = response.get("n").and_then(|v| v.as_u64()) {
        return MongoQueryResult {
            documents: vec![response.clone()],
            affected: n,
            elapsed_ms,
            summary: format!("count={n}, {elapsed_ms}ms"),
        };
    }
    // 写命令：insert/update/delete 直接看 n / nModified
    if let Some(modified) = response.get("nModified").and_then(|v| v.as_u64()) {
        return MongoQueryResult::write(modified, elapsed_ms, "update");
    }
    // 兜底：整个 response 当单文档
    MongoQueryResult::read(vec![response], elapsed_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_cursor_firstbatch() {
        let resp = json!({
            "cursor": {
                "firstBatch": [{"a": 1}, {"a": 2}],
                "id": 0,
                "ns": "db.coll"
            },
            "ok": 1.0
        });
        let r = parse_run_command_response(resp, 10);
        assert_eq!(r.documents.len(), 2);
    }

    #[test]
    fn parse_count_returns_n() {
        let resp = json!({"n": 42, "ok": 1.0});
        let r = parse_run_command_response(resp, 10);
        assert_eq!(r.affected, 42);
        assert!(r.summary.contains("count=42"));
    }

    #[test]
    fn parse_unknown_falls_back_to_single_doc() {
        let resp = json!({"ok": 1.0, "value": "x"});
        let r = parse_run_command_response(resp, 5);
        assert_eq!(r.documents.len(), 1);
    }
}
