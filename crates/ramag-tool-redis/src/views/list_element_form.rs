//! List 元素新增：复用 LinesEditor(List)，按 push_dir 发 LPUSH / RPUSH

use std::sync::Arc;

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, Styled, Window,
    div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use ramag_app::RedisService;
use ramag_domain::entities::ConnectionConfig;
use tracing::{error, info};

use crate::views::lines_editor::{LinesEditor, LinesKind, PushDir};

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
    editor: Entity<LinesEditor>,
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
        let editor = cx.new(|cx| LinesEditor::new(LinesKind::List, window, cx));
        Self {
            service,
            config,
            db,
            key,
            editor,
            state: SubmitState::Idle,
        }
    }

    fn handle_save(&mut self, cx: &mut Context<Self>) {
        let editor_ref = self.editor.read(cx);
        let elems = editor_ref.collect(cx);
        let cmd = match editor_ref.push_dir() {
            PushDir::Tail => "RPUSH",
            PushDir::Head => "LPUSH",
        };
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
                    .child(format!("Key: {}", self.key)),
            )
            .child(self.editor.clone())
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
