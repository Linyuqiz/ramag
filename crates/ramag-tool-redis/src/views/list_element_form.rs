//! List 元素新增弹窗：每行一个元素，可选 [头部 LPUSH | 尾部 RPUSH]

use std::sync::Arc;

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, div, prelude::*, px,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PushDir {
    Head,
    Tail,
}

#[derive(Debug, Clone)]
pub enum ListElementFormEvent {
    Saved,
    Cancelled,
}

#[derive(Debug, Clone)]
enum SubmitState {
    Idle,
    Submitting,
    Failed(String),
}

pub struct ListElementForm {
    service: Arc<RedisService>,
    config: ConnectionConfig,
    db: u8,
    key: String,
    value_input: Entity<InputState>,
    direction: PushDir,
    state: SubmitState,
}

impl EventEmitter<ListElementFormEvent> for ListElementForm {}

impl ListElementForm {
    pub fn new(
        service: Arc<RedisService>,
        config: ConnectionConfig,
        db: u8,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let value_input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("每行一个元素")
        });
        Self {
            service,
            config,
            db,
            key,
            value_input,
            direction: PushDir::Tail,
            state: SubmitState::Idle,
        }
    }

    fn set_dir(&mut self, dir: PushDir, cx: &mut Context<Self>) {
        if self.direction != dir {
            self.direction = dir;
            cx.notify();
        }
    }

    fn handle_save(&mut self, cx: &mut Context<Self>) {
        let raw = self.value_input.read(cx).value().to_string();
        let elems: Vec<String> = raw
            .lines()
            .map(|l| l.trim_end_matches('\r').to_string())
            .filter(|l| !l.is_empty())
            .collect();
        if elems.is_empty() {
            self.state = SubmitState::Failed("至少填写 1 个元素".into());
            cx.notify();
            return;
        }

        self.state = SubmitState::Submitting;
        cx.notify();
        let svc = self.service.clone();
        let config = self.config.clone();
        let db = self.db;
        let key = self.key.clone();
        let cmd = match self.direction {
            PushDir::Head => "LPUSH",
            PushDir::Tail => "RPUSH",
        };
        let mut argv = vec![cmd.to_string(), key];
        argv.extend(elems);
        cx.spawn(async move |this, cx| {
            let result = svc.execute_command(&config, db, argv).await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(_) => {
                    info!(?cmd, "list elements pushed");
                    cx.emit(ListElementFormEvent::Saved);
                }
                Err(e) => {
                    error!(error = %e, "list push failed");
                    this.state = SubmitState::Failed(format!("写入失败：{e}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn handle_cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(ListElementFormEvent::Cancelled);
    }
}

impl Render for ListElementForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let accent = theme.accent;
        let fg = theme.foreground;
        let secondary_bg = theme.secondary;
        let border = theme.border;

        let mut accent_tint = accent;
        accent_tint.a = 0.10;
        let mut accent_border = accent;
        accent_border.a = 0.55;

        let dir = self.direction;
        let dir_btn = |this_dir: PushDir, label: &'static str, id: &'static str| {
            let is_selected = this_dir == dir;
            let mut btn = h_flex()
                .id(SharedString::from(id))
                .flex_1()
                .items_center()
                .justify_center()
                .px(px(8.0))
                .py(px(7.0))
                .rounded_md()
                .border_1()
                .text_sm()
                .child(label.to_string());
            if is_selected {
                btn = btn
                    .bg(accent_tint)
                    .border_color(accent_border)
                    .text_color(accent);
            } else {
                btn = btn
                    .bg(secondary_bg)
                    .border_color(border)
                    .text_color(fg)
                    .cursor_pointer()
                    .hover(move |this| this.border_color(accent_border));
            }
            btn
        };

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
                    .child(format!("Key: {}", self.key)),
            )
            .child(
                v_flex()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(muted_fg)
                            .child("插入位置"),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .gap(px(8.0))
                            .child(dir_btn(PushDir::Head, "头部 LPUSH", "le-head").on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.set_dir(PushDir::Head, cx)
                                }),
                            ))
                            .child(dir_btn(PushDir::Tail, "尾部 RPUSH", "le-tail").on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.set_dir(PushDir::Tail, cx)
                                }),
                            )),
                    ),
            )
            .child(
                v_flex()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(muted_fg)
                            .child("元素（每行一个）"),
                    )
                    .child(
                        div()
                            .w_full()
                            .h(px(180.0))
                            .child(Input::new(&self.value_input)),
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
                                Button::new("le-cancel")
                                    .ghost()
                                    .small()
                                    .label("取消")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.handle_cancel(cx)
                                    })),
                            )
                            .child(
                                Button::new("le-save")
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
