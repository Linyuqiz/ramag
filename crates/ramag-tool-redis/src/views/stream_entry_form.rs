//! Stream 条目新增弹窗：XADD key * field1 value1 field2 value2 ...
//!
//! 用户输入每行 `field value`（与 Hash 表单同款），自动用 `*` 让服务端生成 ID

use std::sync::Arc;

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, Styled, Window,
    div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use ramag_app::RedisService;
use ramag_domain::entities::ConnectionConfig;
use tracing::{error, info};

#[derive(Debug, Clone)]
pub enum StreamEntryFormEvent {
    Saved,
    Cancelled,
}

#[derive(Debug, Clone)]
enum SubmitState {
    Idle,
    Submitting,
    Failed(String),
}

pub struct StreamEntryForm {
    service: Arc<RedisService>,
    config: ConnectionConfig,
    db: u8,
    key: String,
    fields_input: Entity<InputState>,
    state: SubmitState,
}

impl EventEmitter<StreamEntryFormEvent> for StreamEntryForm {}

impl StreamEntryForm {
    pub fn new(
        service: Arc<RedisService>,
        config: ConnectionConfig,
        db: u8,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let fields_input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("每行 `field value`，空格分隔")
        });
        Self {
            service,
            config,
            db,
            key,
            fields_input,
            state: SubmitState::Idle,
        }
    }

    fn handle_save(&mut self, cx: &mut Context<Self>) {
        let raw = self.fields_input.read(cx).value().to_string();
        let pairs = match parse_field_pairs(&raw) {
            Ok(p) => p,
            Err(e) => {
                self.state = SubmitState::Failed(e);
                cx.notify();
                return;
            }
        };
        if pairs.is_empty() {
            self.state = SubmitState::Failed("至少需要 1 个字段".into());
            cx.notify();
            return;
        }

        self.state = SubmitState::Submitting;
        cx.notify();
        let svc = self.service.clone();
        let config = self.config.clone();
        let db = self.db;
        let key = self.key.clone();
        // XADD key * field1 value1 field2 value2 ...
        let mut argv = vec!["XADD".to_string(), key, "*".to_string()];
        for (f, v) in pairs {
            argv.push(f);
            argv.push(v);
        }
        cx.spawn(async move |this, cx| {
            let result = svc.execute_command(&config, db, argv).await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(_) => {
                    info!("stream entry added");
                    cx.emit(StreamEntryFormEvent::Saved);
                }
                Err(e) => {
                    error!(error = %e, "xadd failed");
                    this.state = SubmitState::Failed(format!("写入失败：{e}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn handle_cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(StreamEntryFormEvent::Cancelled);
    }
}

fn parse_field_pairs(raw: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        match line.split_once(' ') {
            Some((f, v)) => {
                let f = f.trim();
                if f.is_empty() {
                    return Err(format!("第 {} 行：field 为空", idx + 1));
                }
                out.push((f.to_string(), v.trim_start().to_string()));
            }
            None => {
                return Err(format!(
                    "第 {} 行格式错误：需要 `field value`（空格分隔）",
                    idx + 1
                ));
            }
        }
    }
    Ok(out)
}

impl Render for StreamEntryForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let border = theme.border;

        let err = match &self.state {
            SubmitState::Idle | SubmitState::Submitting => None,
            SubmitState::Failed(s) => Some(s.clone()),
        };
        let submitting = matches!(self.state, SubmitState::Submitting);

        v_flex()
            .w_full()
            .gap(px(14.0))
            .pt(px(4.0))
            .pb(px(4.0))
            .child(
                div()
                    .text_xs()
                    .text_color(muted_fg)
                    .child(format!("Stream: {} · ID 由服务端生成（*）", self.key)),
            )
            .child(
                v_flex()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(muted_fg)
                            .child("字段（每行 `field value`）"),
                    )
                    .child(
                        div()
                            .w_full()
                            .h(px(180.0))
                            .child(Input::new(&self.fields_input)),
                    ),
            )
            .child(div().h(px(1.0)).bg(border).my(px(2.0)))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(gpui::red())
                            .child(err.unwrap_or_default()),
                    )
                    .child(
                        h_flex()
                            .gap(px(8.0))
                            .flex_none()
                            .child(
                                Button::new("se-stream-cancel")
                                    .ghost()
                                    .small()
                                    .label("取消")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.handle_cancel(cx)
                                    })),
                            )
                            .child(
                                Button::new("se-stream-save")
                                    .primary()
                                    .small()
                                    .label(if submitting { "保存中..." } else { "保存" })
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        if !matches!(this.state, SubmitState::Submitting) {
                                            this.handle_save(cx);
                                        }
                                    })),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic() {
        let r = parse_field_pairs("name alice\nage 30").unwrap();
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn parse_value_with_space() {
        let r = parse_field_pairs("desc hello world").unwrap();
        assert_eq!(r[0].1, "hello world");
    }

    #[test]
    fn parse_missing_value_errors() {
        assert!(parse_field_pairs("only_field").is_err());
    }
}
