//! Stream XADD 弹窗：复用 PairsEditor(Stream)。`XADD key * field value ...`，ID 服务端生成

use std::sync::Arc;

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, Styled, Window,
    div, prelude::*, px,
};
use gpui_component::{ActiveTheme, v_flex};
use ramag_app::RedisService;
use ramag_domain::entities::ConnectionConfig;
use tracing::{error, info};

use crate::views::form_shell::{SubmitState, form_footer};
use crate::views::pairs_editor::{PairsEditor, PairsKind};

#[derive(Debug, Clone)]
pub enum StreamEntryFormEvent {
    Saved,
    Cancelled,
}

pub struct StreamEntryForm {
    service: Arc<RedisService>,
    config: ConnectionConfig,
    db: u8,
    key: String,
    editor: Entity<PairsEditor>,
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
        let editor = cx.new(|cx| PairsEditor::new(PairsKind::Stream, window, cx));
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
        let pairs = match self.editor.read(cx).collect(cx) {
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
                    this.state = SubmitState::Failed(e.write_hint("写入失败"));
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

impl Render for StreamEntryForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let border = theme.border;

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
                            .child("字段"),
                    )
                    .child(self.editor.clone()),
            )
            .child(div().h(px(1.0)).bg(border).my(px(2.0)))
            .child(form_footer(
                "se-stream",
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
