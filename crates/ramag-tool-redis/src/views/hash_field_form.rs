//! Hash 单字段：新增 field+value 或编辑（锁 field）。两种都走 `HSET key field value`

use std::sync::Arc;

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, Styled, Window,
    div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme,
    input::{Input, InputState},
    v_flex,
};
use ramag_app::RedisService;
use ramag_domain::entities::ConnectionConfig;
use tracing::{error, info};

use crate::views::form_shell::{SubmitState, form_footer};

#[derive(Debug, Clone)]
pub enum HashFieldFormMode {
    Add,
    Edit { field: String },
}

#[derive(Debug, Clone)]
pub enum HashFieldFormEvent {
    /// 提交成功，返回 field，让上层重载详情
    Saved {
        field: String,
    },
    Cancelled,
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
                    this.state = SubmitState::Failed(e.write_hint("保存失败"));
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
                            .child(Input::new(&self.value_input).h(px(180.0))),
                    ),
            )
            .child(div().h(px(1.0)).bg(border).my(px(2.0)))
            .child(form_footer(
                "hf",
                "保存",
                &self.state,
                |this, _: &ClickEvent, _, cx| this.handle_cancel(cx),
                |this, _: &ClickEvent, _, cx| {
                    if !this.state.is_submitting() {
                        this.handle_save(cx);
                    }
                },
                cx,
            ))
    }
}
