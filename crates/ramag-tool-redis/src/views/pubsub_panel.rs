//! Pub/Sub 实时面板：订阅 + 实时消息流 + PUBLISH 表单
//!
//! 设计：
//! - 顶部：channel + pattern 输入 + [订阅] [取消所有]
//! - 中部：实时消息流（最新在上，限 200 条防爆）
//! - 底部：channel + payload 输入 + [PUBLISH]
//!
//! 后台任务：用 cx.spawn 持续从 mpsc receiver 读消息，每条 update self.messages

use std::sync::Arc;

use futures::StreamExt;
use gpui::{
    ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    scroll::ScrollableElement as _,
    v_flex,
};
use ramag_app::RedisService;
use ramag_domain::entities::{ConnectionConfig, PubSubMessage};
use tracing::{error, info};

const MAX_MESSAGES: usize = 200;

#[derive(Debug, Clone)]
pub enum PubSubEvent {
    /// 占位（future use）
    #[allow(dead_code)]
    Noop,
}

/// 当前激活的订阅元数据
#[derive(Debug, Clone)]
struct SubscriptionDesc {
    channels: Vec<String>,
    patterns: Vec<String>,
}

pub struct PubSubPanel {
    service: Arc<RedisService>,
    config: ConnectionConfig,
    /// 订阅输入框：逗号分隔多个 channel
    channels_input: Entity<InputState>,
    /// 订阅输入框：逗号分隔多个 pattern（PSUBSCRIBE）
    patterns_input: Entity<InputState>,
    /// PUBLISH 输入：channel
    pub_channel: Entity<InputState>,
    /// PUBLISH 输入：消息体
    pub_message: Entity<InputState>,
    /// 当前激活的订阅描述（None = 未订阅）
    active: Option<SubscriptionDesc>,
    /// 实时消息流（最新在末尾；render 时倒序展示）
    messages: Vec<PubSubMessage>,
    /// 状态文案 / 错误
    status: Option<String>,
    /// 订阅中标志（防止重复触发）
    subscribing: bool,
    /// 后台 task 句柄（drop 即取消，receiver 也会被 drop 进而停止 driver task）
    _task: Option<gpui::Task<()>>,
}

impl EventEmitter<PubSubEvent> for PubSubPanel {}

impl PubSubPanel {
    pub fn new(
        service: Arc<RedisService>,
        config: ConnectionConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let channels_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("channel 列表（逗号分隔，如 foo,bar）")
        });
        let patterns_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("pattern 列表（逗号分隔，如 news.*）")
        });
        let pub_channel = cx.new(|cx| InputState::new(window, cx).placeholder("channel"));
        let pub_message = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("消息体")
        });

        Self {
            service,
            config,
            channels_input,
            patterns_input,
            pub_channel,
            pub_message,
            active: None,
            messages: Vec::new(),
            status: None,
            subscribing: false,
            _task: None,
        }
    }

    fn parse_csv(input: Entity<InputState>, cx: &gpui::App) -> Vec<String> {
        input
            .read(cx)
            .value()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    fn handle_subscribe(&mut self, cx: &mut Context<Self>) {
        if self.subscribing {
            return;
        }
        let channels = Self::parse_csv(self.channels_input.clone(), cx);
        let patterns = Self::parse_csv(self.patterns_input.clone(), cx);
        if channels.is_empty() && patterns.is_empty() {
            self.status = Some("请至少填写一个 channel 或 pattern".into());
            cx.notify();
            return;
        }

        self.subscribing = true;
        self.status = Some("订阅中...".into());
        cx.notify();

        let svc = self.service.clone();
        let config = self.config.clone();
        let channels_clone = channels.clone();
        let patterns_clone = patterns.clone();

        let task = cx.spawn(async move |this, cx| {
            let result = svc
                .subscribe(&config, channels_clone.clone(), patterns_clone.clone())
                .await;
            let mut rx = match result {
                Ok(r) => r,
                Err(e) => {
                    error!(error = %e, "subscribe failed");
                    let _ = this.update(cx, |this, cx| {
                        this.subscribing = false;
                        this.status = Some(format!("订阅失败：{e}"));
                        cx.notify();
                    });
                    return;
                }
            };
            // 订阅成功
            let _ = this.update(cx, |this, cx| {
                this.subscribing = false;
                this.active = Some(SubscriptionDesc {
                    channels: channels_clone,
                    patterns: patterns_clone,
                });
                this.status = Some("已订阅".into());
                cx.notify();
            });

            // 持续读消息流
            while let Some(msg) = rx.next().await {
                let _ = this.update(cx, |this, cx| {
                    if this.messages.len() >= MAX_MESSAGES {
                        this.messages.remove(0);
                    }
                    this.messages.push(msg);
                    cx.notify();
                });
            }
            // 流结束（receiver 被 drop / driver task 退出）
            info!("pubsub stream ended");
            let _ = this.update(cx, |this, cx| {
                this.active = None;
                this.status = Some("订阅已结束".into());
                cx.notify();
            });
        });
        self._task = Some(task);
    }

    fn handle_unsubscribe(&mut self, cx: &mut Context<Self>) {
        // drop _task → spawn 内的 future 被取消 → rx drop → driver task 退出
        self._task = None;
        self.active = None;
        self.status = Some("已取消订阅".into());
        cx.notify();
    }

    fn handle_publish(&mut self, cx: &mut Context<Self>) {
        let channel = self.pub_channel.read(cx).value().trim().to_string();
        let message = self.pub_message.read(cx).value().to_string();
        if channel.is_empty() {
            self.status = Some("请填写 channel".into());
            cx.notify();
            return;
        }
        let svc = self.service.clone();
        let config = self.config.clone();
        cx.spawn(async move |this, cx| {
            let result = svc.publish(&config, &channel, &message).await;
            let _ = this.update(cx, |this, cx| {
                this.status = Some(match result {
                    Ok(n) => format!("发送成功，{n} 个订阅者收到"),
                    Err(e) => format!("发送失败：{e}"),
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn clear_messages(&mut self, cx: &mut Context<Self>) {
        self.messages.clear();
        cx.notify();
    }
}

impl Render for PubSubPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let border = theme.border;
        let bg = theme.background;
        let secondary_bg = theme.secondary;

        // 状态条
        let status_text = match (&self.active, &self.status) {
            (Some(d), _) => format!(
                "✓ 订阅中：channels={:?} patterns={:?} · 已收 {} 条",
                d.channels,
                d.patterns,
                self.messages.len()
            ),
            (None, Some(s)) => s.clone(),
            (None, None) => "未订阅".to_string(),
        };

        // 顶部订阅区
        let subscribe_row = h_flex()
            .w_full()
            .px(px(12.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(border)
            .bg(secondary_bg)
            .gap(px(8.0))
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.channels_input)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.patterns_input)),
            )
            .child(if self.active.is_some() {
                Button::new("ps-unsub")
                    .danger()
                    .small()
                    .label("取消订阅")
                    .on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.handle_unsubscribe(cx)),
                    )
            } else {
                Button::new("ps-sub")
                    .primary()
                    .small()
                    .label(if self.subscribing {
                        "订阅中..."
                    } else {
                        "订阅"
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.handle_subscribe(cx)))
            });

        let status_bar = div()
            .px(px(12.0))
            .py(px(4.0))
            .text_xs()
            .text_color(muted_fg)
            .border_b_1()
            .border_color(border)
            .child(status_text);

        // 消息流（倒序：最新在上）
        let mut msgs_view = v_flex().w_full().gap(px(4.0)).p(px(12.0));
        if self.messages.is_empty() {
            msgs_view = msgs_view.child(div().text_sm().text_color(muted_fg).child("（无消息）"));
        } else {
            for (i, m) in self.messages.iter().rev().enumerate() {
                let row_id = SharedString::from(format!("ps-msg-{}-{}", i, m.received_at_ms));
                let pattern_label = match &m.pattern {
                    Some(p) => format!("（pattern={p}）"),
                    None => String::new(),
                };
                msgs_view = msgs_view.child(
                    v_flex()
                        .id(row_id)
                        .w_full()
                        .px(px(10.0))
                        .py(px(6.0))
                        .border_1()
                        .border_color(border)
                        .rounded(px(4.0))
                        .gap(px(2.0))
                        .child(
                            h_flex()
                                .w_full()
                                .gap(px(8.0))
                                .text_xs()
                                .text_color(muted_fg)
                                .child(div().child(format_timestamp(m.received_at_ms)))
                                .child(
                                    div()
                                        .text_color(theme.accent)
                                        .child(format!("@{}", m.channel)),
                                )
                                .child(div().child(pattern_label)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(fg)
                                .font_family("monospace")
                                .child(m.payload.clone()),
                        ),
                );
            }
        }

        // 底部 PUBLISH 区
        let publish_row = h_flex()
            .w_full()
            .px(px(12.0))
            .py(px(8.0))
            .border_t_1()
            .border_color(border)
            .gap(px(8.0))
            .items_center()
            .child(div().w(px(180.0)).child(Input::new(&self.pub_channel)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.pub_message)),
            )
            .child(
                Button::new("ps-publish")
                    .outline()
                    .small()
                    .label("PUBLISH")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.handle_publish(cx))),
            )
            .child(
                Button::new("ps-clear")
                    .ghost()
                    .small()
                    .label("清空")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.clear_messages(cx))),
            );

        v_flex()
            .size_full()
            .bg(bg)
            .child(subscribe_row)
            .child(status_bar)
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(msgs_view),
            )
            .child(publish_row)
    }
}

fn format_timestamp(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.format("%H:%M:%S%.3f").to_string())
        .unwrap_or_else(|| ms.to_string())
}
