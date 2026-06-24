//! 结果区文档 DML：新增 / 删除 / 编辑。异步执行后 emit Refresh 重跑命令刷新结果。
//! toast 经 pending_notification 在下次 render 推送（与 dbclient::result_panel 同款）

use gpui::{ClickEvent, Context, Entity, SharedString, Window, div, prelude::*, px};
use gpui_component::{
    ActiveTheme, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    notification::Notification,
    v_flex,
};
use serde_json::Value;

use super::{ResultEvent, ResultPanel};

impl ResultPanel {
    /// 弹「新增文档」：按当前结果的字段逐项填写（对齐 dbclient 按列填）；确认后 insert_one
    pub(crate) fn open_insert_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(coll) = self.target_collection.clone() else {
            return;
        };
        // 字段模板：当前结果首个文档的顶层字段（排除 _id，让 mongo 自动生成）
        let fields: Vec<String> = self
            .result
            .as_ref()
            .and_then(|r| r.documents.first())
            .and_then(|d| d.as_object())
            .map(|m| m.keys().filter(|k| k.as_str() != "_id").cloned().collect())
            .unwrap_or_default();
        if fields.is_empty() {
            return self.notify_error(
                "无字段模板：请先查询出该 collection 的文档，或在编辑器用 insert 命令新增"
                    .to_string(),
                cx,
            );
        }
        let inputs: Vec<(String, Entity<InputState>)> = fields
            .iter()
            .map(|f| {
                (
                    f.clone(),
                    cx.new(|c| {
                        InputState::new(window, c).placeholder("值（JSON / 文本，留空跳过）")
                    }),
                )
            })
            .collect();
        let panel = cx.entity().clone();
        let title = SharedString::from(format!("新增文档 → {coll}"));
        window.open_dialog(cx, move |dialog, _, _| {
            let panel_apply = panel.clone();
            let inputs_apply = inputs.clone();
            let inputs_content = inputs.clone();
            let cancel = Button::new("mongo-insert-cancel")
                .ghost()
                .small()
                .label("取消")
                .on_click(move |_: &ClickEvent, window, app| window.close_dialog(app));
            let apply = Button::new("mongo-insert-apply")
                .primary()
                .small()
                .label("插入")
                .on_click(move |_: &ClickEvent, window, app| {
                    let pairs: Vec<(String, String)> = inputs_apply
                        .iter()
                        .map(|(f, inp)| (f.clone(), inp.read(app).value().to_string()))
                        .collect();
                    panel_apply.update(app, |this, cx| this.do_insert_fields(pairs, cx));
                    window.close_dialog(app);
                });
            dialog
                .title(title.clone())
                .width(px(520.0))
                .margin_top(px(100.0))
                .content(move |content, _, cx| {
                    let muted = cx.theme().muted_foreground;
                    let mut col = v_flex().w_full().gap(px(10.0));
                    for (field, input) in &inputs_content {
                        col = col.child(
                            v_flex()
                                .w_full()
                                .gap(px(2.0))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(SharedString::from(field.clone())),
                                )
                                .child(Input::new(input).small()),
                        );
                    }
                    content.child(col)
                })
                .footer(dialog_footer(cancel, apply))
        });
    }

    /// 表单字段组装成文档 → insert_one（留空字段跳过；值按 JSON 解析，失败当字符串）
    fn do_insert_fields(&mut self, pairs: Vec<(String, String)>, cx: &mut Context<Self>) {
        let mut map = serde_json::Map::new();
        for (field, raw) in pairs {
            if raw.trim().is_empty() {
                continue;
            }
            let val = match serde_json::from_str::<Value>(raw.trim()) {
                Ok(v) => v,
                Err(_) => Value::String(raw),
            };
            map.insert(field, val);
        }
        if map.is_empty() {
            return self.notify_error("未填写任何字段".to_string(), cx);
        }
        self.do_insert_doc(Value::Object(map), cx);
    }

    /// 异步 insert_one；成功 emit Refresh + toast
    fn do_insert_doc(&mut self, doc: Value, cx: &mut Context<Self>) {
        let (Some(svc), Some(conf), Some(coll)) = (
            self.service.clone(),
            self.config.clone(),
            self.target_collection.clone(),
        ) else {
            return;
        };
        let db = self.database.clone();
        cx.spawn(async move |this, cx| {
            let r = svc.insert_one(&conf, &db, &coll, doc).await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(id) => {
                        this.pending_notification = Some(
                            Notification::success(format!("已插入文档 _id={id}")).autohide(true),
                        );
                        cx.emit(ResultEvent::Refresh);
                    }
                    Err(e) => {
                        this.pending_notification =
                            Some(Notification::error(e.write_hint("插入失败")).autohide(true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 同步路径错误 toast
    pub(crate) fn notify_error(&mut self, msg: String, cx: &mut Context<Self>) {
        self.pending_notification = Some(Notification::error(msg).autohide(true));
        cx.notify();
    }

    /// 弹删除确认；确认后对勾选行按 _id 逐个 delete_one
    pub(crate) fn open_delete_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(result) = self.result.as_ref() else {
            return;
        };
        let ids: Vec<Value> = self
            .selected_rows
            .iter()
            .filter_map(|&i| result.documents.get(i))
            .filter_map(|d| d.get("_id").cloned())
            .collect();
        if ids.is_empty() {
            return self.notify_error("勾选的文档缺少 _id，无法删除".to_string(), cx);
        }
        let n = ids.len();
        let coll = self.target_collection.clone().unwrap_or_default();
        let panel = cx.entity().clone();
        let title = SharedString::from(format!("删除 {n} 个文档？"));
        window.open_dialog(cx, move |dialog, _, _| {
            let panel_apply = panel.clone();
            let ids_apply = ids.clone();
            let coll_hint = coll.clone();
            let cancel = Button::new("mongo-del-cancel")
                .ghost()
                .small()
                .label("取消")
                .on_click(move |_: &ClickEvent, window, app| window.close_dialog(app));
            let apply = Button::new("mongo-del-apply")
                .danger()
                .small()
                .label("删除")
                .on_click(move |_: &ClickEvent, window, app| {
                    let ids = ids_apply.clone();
                    panel_apply.update(app, |this, cx| this.do_delete_async(ids, cx));
                    window.close_dialog(app);
                });
            dialog
                .title(title.clone())
                .width(px(460.0))
                .margin_top(px(160.0))
                .content(move |content, _, cx| {
                    let muted = cx.theme().muted_foreground;
                    content.child(div().text_sm().text_color(muted).child(SharedString::from(
                        format!("将从「{coll_hint}」按 _id 逐个删除，操作不可撤销"),
                    )))
                })
                .footer(dialog_footer(cancel, apply))
        });
    }

    /// 异步逐个 delete_one；完成后 emit Refresh
    fn do_delete_async(&mut self, ids: Vec<Value>, cx: &mut Context<Self>) {
        let (Some(svc), Some(conf), Some(coll)) = (
            self.service.clone(),
            self.config.clone(),
            self.target_collection.clone(),
        ) else {
            return;
        };
        let db = self.database.clone();
        cx.spawn(async move |this, cx| {
            let mut ok = 0usize;
            let mut failed: Option<ramag_domain::error::DomainError> = None;
            for id in ids {
                let filter = serde_json::json!({ "_id": id });
                match svc.delete_one(&conf, &db, &coll, &filter).await {
                    Ok(_) => ok += 1,
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                }
            }
            let _ = this.update(cx, |this, cx| {
                match failed {
                    Some(e) => {
                        this.pending_notification = Some(
                            Notification::error(e.write_hint(&format!("删除失败（已删 {ok} 个）")))
                                .autohide(true),
                        )
                    }
                    None => {
                        this.pending_notification = Some(
                            Notification::success(format!("已删除 {ok} 个文档")).autohide(true),
                        );
                        cx.emit(ResultEvent::Refresh);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// 弹窗底部按钮条：右对齐「取消 + 主操作」，两个 dialog 共用同款布局
fn dialog_footer(cancel: Button, apply: Button) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_center()
        .justify_end()
        .gap(px(8.0))
        .child(cancel)
        .child(apply)
}
