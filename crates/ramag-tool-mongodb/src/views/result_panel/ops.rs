//! 结果区文档 DML：新增 / 删除 / 编辑。异步执行后 emit Refresh 重跑命令刷新结果。
//! toast 经 pending_notification 在下次 render 推送（与 dbclient::result_panel 同款）

use std::path::PathBuf;

use futures::channel::oneshot;
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

use super::flatten::FlatTable;
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
                .footer(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_end()
                        .gap(px(8.0))
                        .child(cancel)
                        .child(apply),
                )
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
                            Some(Notification::error(format!("插入失败：{e}")).autohide(true));
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

    /// 导出当前结果：as_csv=true 导 CSV（基于扁平表格），否则导 JSON（原始文档）
    pub(crate) fn export_documents(&mut self, as_csv: bool, cx: &mut Context<Self>) {
        let Some(result) = self.result.as_ref() else {
            return self.notify_error("无可导出的结果".to_string(), cx);
        };
        if result.documents.is_empty() {
            return self.notify_error("结果为空，无需导出".to_string(), cx);
        }
        let (content, ext) = if as_csv {
            match &self.table {
                Some(t) => (flat_to_csv(t), "csv"),
                None => return self.notify_error("无表格数据可导出 CSV".to_string(), cx),
            }
        } else {
            (
                serde_json::to_string_pretty(&result.documents).unwrap_or_default(),
                "json",
            )
        };
        let coll = self
            .target_collection
            .clone()
            .unwrap_or_else(|| "export".to_string());
        let name = format!("{coll}.{ext}");
        // rfd 保存框是阻塞的：放 std::thread 跑，结果经 oneshot 回主线程（与 dbclient 同款）
        let (tx, rx) = oneshot::channel::<ExportOutcome>();
        std::thread::spawn(move || {
            let path = rfd::FileDialog::new()
                .set_file_name(&name)
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
                this.pending_notification = Some(match outcome {
                    ExportOutcome::Saved(p) => Notification::success(
                        p.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "导出完成".to_string()),
                    )
                    .title("导出成功")
                    .autohide(true),
                    ExportOutcome::Cancelled => Notification::info("已取消导出").autohide(true),
                    ExportOutcome::Failed(e) => {
                        Notification::error(e).title("导出失败").autohide(true)
                    }
                });
                cx.notify();
            });
        })
        .detach();
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
                .footer(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_end()
                        .gap(px(8.0))
                        .child(cancel)
                        .child(apply),
                )
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
            let mut failed: Option<String> = None;
            for id in ids {
                let filter = serde_json::json!({ "_id": id });
                match svc.delete_one(&conf, &db, &coll, &filter).await {
                    Ok(_) => ok += 1,
                    Err(e) => {
                        failed = Some(e.to_string());
                        break;
                    }
                }
            }
            let _ = this.update(cx, |this, cx| {
                match failed {
                    Some(e) => {
                        this.pending_notification = Some(
                            Notification::error(format!("删除失败（已删 {ok} 个）：{e}"))
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

    /// 双击单元格编辑：输入新值（按 JSON 解析）→ update_one $set（dotted path）
    pub(crate) fn open_cell_edit_dialog(
        &self,
        id: Value,
        path: String,
        current: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|c| InputState::new(window, c).default_value(current));
        input.update(cx, |s, c| s.focus(window, c));
        let panel = cx.entity().clone();
        let title = SharedString::from(format!("编辑字段 {path}"));
        window.open_dialog(cx, move |dialog, _, _| {
            let panel_apply = panel.clone();
            let input_apply = input.clone();
            let input_content = input.clone();
            let id_apply = id.clone();
            let path_apply = path.clone();
            let cancel = Button::new("mongo-edit-cancel")
                .ghost()
                .small()
                .label("取消")
                .on_click(move |_: &ClickEvent, window, app| window.close_dialog(app));
            let apply = Button::new("mongo-edit-apply")
                .primary()
                .small()
                .label("保存")
                .on_click(move |_: &ClickEvent, window, app| {
                    let raw = input_apply.read(app).value().to_string();
                    let id = id_apply.clone();
                    let path = path_apply.clone();
                    panel_apply.update(app, |this, cx| this.do_update_async(id, path, raw, cx));
                    window.close_dialog(app);
                });
            dialog
                .title(title.clone())
                .width(px(520.0))
                .margin_top(px(150.0))
                .content(move |content, _, cx| {
                    let muted = cx.theme().muted_foreground;
                    content.child(
                        div()
                            .w_full()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .pb(px(6.0))
                                    .child("输入按 JSON 解析：123→数字、true→布尔、其它→字符串"),
                            )
                            .child(Input::new(&input_content).h(px(220.0))),
                    )
                })
                .footer(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_end()
                        .gap(px(8.0))
                        .child(cancel)
                        .child(apply),
                )
        });
    }

    /// 异步 update_one：filter {_id} + $set {dotted path: 新值}
    fn do_update_async(&mut self, id: Value, path: String, raw: String, cx: &mut Context<Self>) {
        let new_val: Value = match serde_json::from_str::<Value>(&raw) {
            Ok(v) => v,
            Err(_) => Value::String(raw),
        };
        let (Some(svc), Some(conf), Some(coll)) = (
            self.service.clone(),
            self.config.clone(),
            self.target_collection.clone(),
        ) else {
            return;
        };
        let db = self.database.clone();
        let filter = serde_json::json!({ "_id": id });
        let mut set = serde_json::Map::new();
        set.insert(path, new_val);
        let mut update = serde_json::Map::new();
        update.insert("$set".to_string(), Value::Object(set));
        let update = Value::Object(update);
        cx.spawn(async move |this, cx| {
            let r = svc.update_one(&conf, &db, &coll, &filter, &update).await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(res) if res.affected == 0 => {
                        this.pending_notification = Some(
                            Notification::warning(
                                "未匹配到文档（0 条更新）：结果集可能无 _id 或 _id 类型不符"
                                    .to_string(),
                            )
                            .autohide(true),
                        );
                    }
                    Ok(res) => {
                        this.pending_notification = Some(
                            Notification::success(format!("已更新 {} 条文档", res.affected))
                                .autohide(true),
                        );
                        cx.emit(ResultEvent::Refresh);
                    }
                    Err(e) => {
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

/// rfd 文件保存结果（线程 → 主线程）
enum ExportOutcome {
    Saved(PathBuf),
    Cancelled,
    Failed(String),
}

/// FlatTable → CSV（列头 path + 行，逗号/引号/换行转义）
fn flat_to_csv(table: &FlatTable) -> String {
    let mut out = String::new();
    let header: Vec<String> = table.columns.iter().map(|c| csv_escape(&c.path)).collect();
    out.push_str(&header.join(","));
    out.push('\n');
    for row in &table.rows {
        let cells: Vec<String> = row.iter().map(|c| csv_escape(&c.text)).collect();
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    out
}

/// CSV 字段转义：含逗号 / 引号 / 换行时用双引号包裹，内部引号翻倍
fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
