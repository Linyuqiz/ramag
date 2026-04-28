//! TTL 编辑弹窗：设置秒数 / 永久（PERSIST）

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
pub enum TtlEditEvent {
    /// TTL 已更新（None=永久）
    Updated(String),
    Cancelled,
}

#[derive(Debug, Clone)]
enum SubmitState {
    Idle,
    Submitting,
    Failed(String),
}

pub struct TtlEditForm {
    service: Arc<RedisService>,
    config: ConnectionConfig,
    db: u8,
    key: String,
    /// 当前 TTL（毫秒）：用于初始化 input；-1 = 永久
    initial_ttl_ms: Option<i64>,
    secs_input: Entity<InputState>,
    state: SubmitState,
}

impl EventEmitter<TtlEditEvent> for TtlEditForm {}

impl TtlEditForm {
    pub fn new(
        service: Arc<RedisService>,
        config: ConnectionConfig,
        db: u8,
        key: String,
        initial_ttl_ms: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // 初始值：>0 ms 显示为秒；其他空
        let initial_secs = match initial_ttl_ms {
            Some(ms) if ms > 0 => (ms / 1000).to_string(),
            _ => String::new(),
        };
        let secs_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("秒数（如 3600）")
                .default_value(initial_secs)
        });
        Self {
            service,
            config,
            db,
            key,
            initial_ttl_ms,
            secs_input,
            state: SubmitState::Idle,
        }
    }

    fn handle_set(&mut self, cx: &mut Context<Self>) {
        let raw = self.secs_input.read(cx).value().trim().to_string();
        if raw.is_empty() {
            self.state = SubmitState::Failed("请输入秒数（或点击「设为永久」）".into());
            cx.notify();
            return;
        }
        let secs: i64 = match raw.parse() {
            Ok(n) if n > 0 => n,
            _ => {
                self.state = SubmitState::Failed("秒数必须是正整数".into());
                cx.notify();
                return;
            }
        };
        self.submit_ttl(Some(secs), cx);
    }

    fn handle_persist(&mut self, cx: &mut Context<Self>) {
        self.submit_ttl(None, cx);
    }

    fn submit_ttl(&mut self, ttl_secs: Option<i64>, cx: &mut Context<Self>) {
        self.state = SubmitState::Submitting;
        cx.notify();
        let svc = self.service.clone();
        let config = self.config.clone();
        let db = self.db;
        let key = self.key.clone();
        let label = match ttl_secs {
            Some(s) => format!("{s}s"),
            None => "永久".to_string(),
        };
        cx.spawn(async move |this, cx| {
            let result = svc.set_ttl(&config, db, &key, ttl_secs).await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(true) => {
                    info!(?key, ?ttl_secs, "ttl updated");
                    cx.emit(TtlEditEvent::Updated(label));
                }
                Ok(false) => {
                    error!(?key, "ttl update returned false (key may be gone)");
                    this.state = SubmitState::Failed("Key 不存在或操作未生效".into());
                    cx.notify();
                }
                Err(e) => {
                    error!(error = %e, "ttl update failed");
                    this.state = SubmitState::Failed(format!("更新失败：{e}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn handle_cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(TtlEditEvent::Cancelled);
    }
}

impl Render for TtlEditForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let border = theme.border;

        let current_label = match self.initial_ttl_ms {
            Some(-1) => "永久（无 TTL）".to_string(),
            Some(-2) => "Key 不存在".to_string(),
            Some(ms) if ms >= 0 => format!("当前剩余 {} ms", ms),
            _ => "未知".to_string(),
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
                v_flex()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(format!("Key: {}", self.key)),
                    )
                    .child(div().text_xs().text_color(muted_fg).child(current_label)),
            )
            .child(
                v_flex()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(muted_fg)
                            .child("新 TTL（秒）"),
                    )
                    .child(div().w_full().child(Input::new(&self.secs_input))),
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
                                Button::new("ttl-persist")
                                    .ghost()
                                    .small()
                                    .label("设为永久")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        if !matches!(this.state, SubmitState::Submitting) {
                                            this.handle_persist(cx);
                                        }
                                    })),
                            )
                            .child(
                                Button::new("ttl-cancel")
                                    .ghost()
                                    .small()
                                    .label("取消")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.handle_cancel(cx);
                                    })),
                            )
                            .child(
                                Button::new("ttl-set")
                                    .primary()
                                    .small()
                                    .label(if submitting { "保存中..." } else { "保存" })
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        if !matches!(this.state, SubmitState::Submitting) {
                                            this.handle_set(cx);
                                        }
                                    })),
                            ),
                    ),
            )
    }
}
