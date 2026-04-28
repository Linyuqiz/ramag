//! ZSet 元素 add / edit 弹窗
//!
//! - Add 模式：新增 score + member（member 不锁）
//! - EditScore 模式：member 锁定，仅改 score → ZADD 覆盖

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
pub enum ZSetElementFormMode {
    Add,
    EditScore { member: String },
}

#[derive(Debug, Clone)]
pub enum ZSetElementFormEvent {
    Saved,
    Cancelled,
}

#[derive(Debug, Clone)]
enum SubmitState {
    Idle,
    Submitting,
    Failed(String),
}

pub struct ZSetElementForm {
    service: Arc<RedisService>,
    config: ConnectionConfig,
    db: u8,
    key: String,
    mode: ZSetElementFormMode,
    score_input: Entity<InputState>,
    member_input: Entity<InputState>,
    state: SubmitState,
}

impl EventEmitter<ZSetElementFormEvent> for ZSetElementForm {}

impl ZSetElementForm {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        service: Arc<RedisService>,
        config: ConnectionConfig,
        db: u8,
        key: String,
        mode: ZSetElementFormMode,
        initial_score: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (initial_member, _is_edit) = match &mode {
            ZSetElementFormMode::Add => (String::new(), false),
            ZSetElementFormMode::EditScore { member } => (member.clone(), true),
        };
        let score_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("数字（如 3.14）")
                .default_value(initial_score)
        });
        let member_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("成员名")
                .default_value(initial_member)
        });
        Self {
            service,
            config,
            db,
            key,
            mode,
            score_input,
            member_input,
            state: SubmitState::Idle,
        }
    }

    fn handle_save(&mut self, cx: &mut Context<Self>) {
        let score_raw = self.score_input.read(cx).value().trim().to_string();
        if score_raw.is_empty() {
            self.state = SubmitState::Failed("请填写 score".into());
            cx.notify();
            return;
        }
        if score_raw.parse::<f64>().is_err() {
            self.state = SubmitState::Failed("score 必须是数字".into());
            cx.notify();
            return;
        }
        let member = match &self.mode {
            ZSetElementFormMode::EditScore { member } => member.clone(),
            ZSetElementFormMode::Add => self.member_input.read(cx).value().trim().to_string(),
        };
        if member.is_empty() {
            self.state = SubmitState::Failed("请填写成员名".into());
            cx.notify();
            return;
        }

        self.state = SubmitState::Submitting;
        cx.notify();
        let svc = self.service.clone();
        let config = self.config.clone();
        let db = self.db;
        let key = self.key.clone();
        let argv = vec!["ZADD".to_string(), key, score_raw, member.clone()];
        cx.spawn(async move |this, cx| {
            let result = svc.execute_command(&config, db, argv).await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(_) => {
                    info!(?member, "zset member saved");
                    cx.emit(ZSetElementFormEvent::Saved);
                }
                Err(e) => {
                    error!(error = %e, "zadd failed");
                    this.state = SubmitState::Failed(format!("写入失败：{e}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn handle_cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(ZSetElementFormEvent::Cancelled);
    }
}

impl Render for ZSetElementForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let border = theme.border;

        let is_edit = matches!(self.mode, ZSetElementFormMode::EditScore { .. });
        let err = match &self.state {
            SubmitState::Idle | SubmitState::Submitting => None,
            SubmitState::Failed(s) => Some(s.clone()),
        };
        let submitting = matches!(self.state, SubmitState::Submitting);

        let member_block = if is_edit {
            v_flex()
                .gap(px(6.0))
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(muted_fg)
                        .child("成员（不可修改）"),
                )
                .child(
                    div()
                        .w_full()
                        .opacity(0.6)
                        .child(Input::new(&self.member_input).disabled(true)),
                )
        } else {
            v_flex()
                .gap(px(6.0))
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(muted_fg)
                        .child("成员"),
                )
                .child(div().w_full().child(Input::new(&self.member_input)))
        };

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
                            .child("Score"),
                    )
                    .child(div().w_full().child(Input::new(&self.score_input))),
            )
            .child(member_block)
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
                                Button::new("ze-cancel")
                                    .ghost()
                                    .small()
                                    .label("取消")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.handle_cancel(cx)
                                    })),
                            )
                            .child(
                                Button::new("ze-save")
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
