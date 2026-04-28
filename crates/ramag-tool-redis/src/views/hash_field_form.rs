//! Hash 单字段编辑 / 新增弹窗
//!
//! - 新增模式：填 field + value → HSET key field value
//! - 编辑模式：field 锁定，仅改 value → HSET key field value（HSET 等价 update）

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
pub enum HashFieldFormMode {
    Add,
    Edit { field: String },
}

#[derive(Debug, Clone)]
pub enum HashFieldFormEvent {
    /// 提交成功，返回 (field, value)，让上层重载详情
    Saved {
        field: String,
    },
    Cancelled,
}

#[derive(Debug, Clone)]
enum SubmitState {
    Idle,
    Submitting,
    Failed(String),
}

pub struct HashFieldForm {
    service: Arc<RedisService>,
    config: ConnectionConfig,
    db: u8,
    key: String,
    mode: HashFieldFormMode,
    field_input: Entity<InputState>,
    value_input: Entity<InputState>,
    state: SubmitState,
}

impl EventEmitter<HashFieldFormEvent> for HashFieldForm {}

impl HashFieldForm {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        service: Arc<RedisService>,
        config: ConnectionConfig,
        db: u8,
        key: String,
        mode: HashFieldFormMode,
        initial_value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_field = match &mode {
            HashFieldFormMode::Add => String::new(),
            HashFieldFormMode::Edit { field } => field.clone(),
        };
        let field_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("字段名（如 name）")
                .default_value(initial_field)
        });
        let value_input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("字段值（任意文本，可多行）")
                .default_value(initial_value)
        });
        Self {
            service,
            config,
            db,
            key,
            mode,
            field_input,
            value_input,
            state: SubmitState::Idle,
        }
    }

    fn handle_save(&mut self, cx: &mut Context<Self>) {
        let field = match &self.mode {
            HashFieldFormMode::Edit { field } => field.clone(),
            HashFieldFormMode::Add => self.field_input.read(cx).value().trim().to_string(),
        };
        if field.is_empty() {
            self.state = SubmitState::Failed("请填写字段名".into());
            cx.notify();
            return;
        }
        let value = self.value_input.read(cx).value().to_string();

        self.state = SubmitState::Submitting;
        cx.notify();
        let svc = self.service.clone();
        let config = self.config.clone();
        let db = self.db;
        let key = self.key.clone();
        let argv = vec!["HSET".to_string(), key, field.clone(), value];
        cx.spawn(async move |this, cx| {
            let result = svc.execute_command(&config, db, argv).await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(_) => {
                    info!(?field, "hash field saved");
                    cx.emit(HashFieldFormEvent::Saved { field });
                }
                Err(e) => {
                    error!(error = %e, "save hash field failed");
                    this.state = SubmitState::Failed(format!("保存失败：{e}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn handle_cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(HashFieldFormEvent::Cancelled);
    }
}

impl Render for HashFieldForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let border = theme.border;

        let is_edit = matches!(self.mode, HashFieldFormMode::Edit { .. });
        let err = match &self.state {
            SubmitState::Idle | SubmitState::Submitting => None,
            SubmitState::Failed(s) => Some(s.clone()),
        };
        let submitting = matches!(self.state, SubmitState::Submitting);

        // 编辑模式下 field input 禁用（视觉上灰显）
        let field_block = if is_edit {
            v_flex()
                .gap(px(6.0))
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(muted_fg)
                        .child("字段（不可修改）"),
                )
                .child(
                    div()
                        .w_full()
                        .opacity(0.6)
                        .child(Input::new(&self.field_input).disabled(true)),
                )
        } else {
            v_flex()
                .gap(px(6.0))
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(muted_fg)
                        .child("字段名"),
                )
                .child(div().w_full().child(Input::new(&self.field_input)))
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
            .child(field_block)
            .child(
                v_flex()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(muted_fg)
                            .child("值"),
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
                                Button::new("hf-cancel")
                                    .ghost()
                                    .small()
                                    .label("取消")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.handle_cancel(cx);
                                    })),
                            )
                            .child(
                                Button::new("hf-save")
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
