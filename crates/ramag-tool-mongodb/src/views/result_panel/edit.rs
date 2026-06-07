//! 单元格编辑：双击 → 输入新值 → update_one $set（dotted path）。
//! 按列原始 BSON 类型还原写入值，避免 oid/date/decimal 被降级成字符串 / 浮点。

use gpui::{ClickEvent, Context, SharedString, Window, div, prelude::*, px};
use gpui_component::{
    ActiveTheme, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    notification::Notification,
};
use serde_json::Value;

use super::{ResultEvent, ResultPanel};

impl ResultPanel {
    /// 双击单元格编辑：输入新值 → update_one $set（dotted path）。
    /// kind 是该列原始 BSON 类型，用于保存时按类型还原（oid/date/decimal 不降级成字符串）
    pub(crate) fn open_cell_edit_dialog(
        &self,
        id: Value,
        path: String,
        kind: &'static str,
        current: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // multi_line：值可能含换行（GPUI 单行 shape_line 不接受 \n），且本对话框是 220px 多行编辑框
        let input = cx.new(|c| {
            InputState::new(window, c)
                .multi_line(true)
                .default_value(current)
        });
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
                    panel_apply.update(app, |this, cx| {
                        this.do_update_async(id, path, kind, raw, cx)
                    });
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

    /// 异步 update_one：filter {_id} + $set {dotted path: 新值（按列 kind 还原 BSON 类型）}
    fn do_update_async(
        &mut self,
        id: Value,
        path: String,
        kind: &'static str,
        raw: String,
        cx: &mut Context<Self>,
    ) {
        let new_val = value_for_kind(kind, raw);
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
                                "未匹配到文档：该行无 _id，或当前 collection 与所选库不一致"
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

/// 按列原始 BSON 类型把单元格编辑文本还原为写入值：
/// 特殊类型（oid/date/decimal）包回 Extended JSON，避免 $set 把它降级成字符串 / 浮点；
/// 其余按 JSON 解析（123→数字 / true→布尔 / 其它→字符串），保留「可改类型」的灵活性。
/// 注：date 文本需为 ISO8601（结果集 relaxed Extended JSON 形态即 ISO），否则 driver 转换报错
fn value_for_kind(kind: &str, raw: String) -> Value {
    match kind {
        "oid" => serde_json::json!({ "$oid": raw }),
        "date" => serde_json::json!({ "$date": raw }),
        "decimal" => serde_json::json!({ "$numberDecimal": raw }),
        _ => match serde_json::from_str::<Value>(&raw) {
            Ok(v) => v,
            Err(_) => Value::String(raw),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::value_for_kind;
    use serde_json::json;

    #[test]
    fn special_kinds_wrap_extjson() {
        // oid/date/decimal 必须包回 Extended JSON，否则会被写成普通字符串/浮点
        assert_eq!(
            value_for_kind("oid", "507f1f77bcf86cd799439011".into()),
            json!({"$oid": "507f1f77bcf86cd799439011"})
        );
        assert_eq!(
            value_for_kind("date", "2024-01-01T00:00:00Z".into()),
            json!({"$date": "2024-01-01T00:00:00Z"})
        );
        assert_eq!(
            value_for_kind("decimal", "100.50".into()),
            json!({"$numberDecimal": "100.50"})
        );
    }

    #[test]
    fn scalar_kinds_parse_json_or_string() {
        assert_eq!(value_for_kind("int", "42".into()), json!(42));
        assert_eq!(value_for_kind("bool", "true".into()), json!(true));
        assert_eq!(value_for_kind("text", "alice".into()), json!("alice"));
    }
}
