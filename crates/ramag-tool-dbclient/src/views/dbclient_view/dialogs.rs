//! DbClientView 的弹窗与异步处理：连接表单 / 删除确认 / 异步删除

use gpui::{AppContext as _, Context, Entity, ParentElement, Styled, Window, px};
use gpui_component::WindowExt as _;
use ramag_domain::entities::{ConnectionConfig, ConnectionId, DriverKind};
use tracing::{error, info};

use crate::views::connection_form::{self, ConnectionFormPanel, FormEvent};

use super::{CenterMode, DbClientView};

impl DbClientView {
    pub(super) fn open_form_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let svc = self.service.clone();
        let redis_svc = self.redis_service.clone();
        let form = cx.new(|cx| ConnectionFormPanel::new_create(svc, redis_svc, window, cx));
        self.subscribe_form_and_open_dialog(form, window, cx);
    }

    pub(super) fn open_form_edit(
        &mut self,
        conn: ConnectionConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let svc = self.service.clone();
        let redis_svc = self.redis_service.clone();
        let form = cx.new(|cx| ConnectionFormPanel::new_edit(svc, redis_svc, conn, window, cx));
        self.subscribe_form_and_open_dialog(form, window, cx);
    }

    /// 订阅表单事件并通过 dialog 系统弹出
    fn subscribe_form_and_open_dialog(
        &mut self,
        form: Entity<ConnectionFormPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sub = cx.subscribe_in(&form, window, Self::on_form_event);
        self._subscriptions.push(sub);

        // 取一次 dialog 标题（mode 在 form 创建时就定了；不显示 driver 名，由表单内选择行体现）
        let title = {
            let f = form.read(cx);
            connection_form::dialog_title(f.mode()).to_string()
        };
        let form_for_dialog = form.clone();

        window.open_dialog(cx, move |dialog, _w, _app| {
            let form = form_for_dialog.clone();
            let title = title.clone();
            dialog
                .title(title)
                .close_button(true)
                .w(px(720.0))
                .p(px(24.0))
                .content(move |content, _, _| content.child(form.clone()))
        });
    }

    fn on_form_event(
        &mut self,
        _form: &Entity<ConnectionFormPanel>,
        event: &FormEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            FormEvent::Saved(conn) => {
                info!("connection saved, refreshing picker");
                window.close_dialog(cx);
                self.picker.update(cx, |p, cx| p.refresh(cx));
                // 失效 driver 内的连接池缓存：池按 ConnectionId 索引，旧 config 建的池
                // 还指向旧 host/db，必须丢弃，下次访问按新 config 重建
                match conn.driver {
                    DriverKind::Mysql | DriverKind::Postgres => {
                        self.service.evict_pool(conn);
                    }
                    DriverKind::Redis => {
                        self.redis_service.evict_pool(&conn.id);
                    }
                }
                // 编辑场景：若该连接有正在打开的 Session，旧 config 已过期（如 database 改了）→
                // 静默关闭旧 Session，避免它继续基于旧池跑查询误导用户
                if let Some(idx) = self
                    .sessions
                    .iter()
                    .position(|s| s.config(cx).id == conn.id)
                {
                    self.sessions.remove(idx);
                    match self.active_session {
                        Some(active) if active == idx => {
                            if self.sessions.is_empty() {
                                self.active_session = None;
                                self.center = CenterMode::ConnectionPicker;
                            } else {
                                let new_active = active.min(self.sessions.len() - 1);
                                self.active_session = Some(new_active);
                            }
                        }
                        Some(active) if active > idx => {
                            self.active_session = Some(active - 1);
                        }
                        _ => {}
                    }
                }
                cx.notify();
            }
            FormEvent::Cancelled => {
                window.close_dialog(cx);
            }
        }
    }

    /// 弹出删除确认对话框；用户点「删除」后才真正执行 handle_delete
    pub(super) fn confirm_delete(
        &mut self,
        id: ConnectionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 找到对应连接名（用于描述文案）；找不到就 fallback 到 id 后 6 位
        let conn_name = self
            .picker
            .read(cx)
            .connections()
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| {
                let s = id.to_string();
                s.chars().rev().take(6).collect::<String>()
            });

        let view = cx.entity();
        ramag_ui::open_confirm(
            "删除连接？",
            format!("确定要删除连接「{conn_name}」吗？此操作不可撤销。"),
            "删除",
            true,
            move |_window, app| {
                view.update(app, |this, cx| {
                    this.handle_delete(id, cx);
                });
            },
            window,
            cx,
        );
    }

    fn handle_delete(&mut self, id: ConnectionId, cx: &mut Context<Self>) {
        let svc = self.service.clone();
        let id_for_async = id.clone();
        cx.spawn(async move |this, cx| {
            let result = svc.delete(&id_for_async).await;
            let _ = this.update(cx, |this, cx| {
                if let Err(e) = result {
                    error!(error = %e, "delete connection failed");
                    return;
                }
                // 关闭对应 session（如果开着）
                let to_close: Vec<usize> = this
                    .sessions
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.config(cx).id == id_for_async)
                    .map(|(i, _)| i)
                    .collect();
                for idx in to_close.into_iter().rev() {
                    this.close_session(idx, cx);
                }
                this.picker.update(cx, |p, cx| p.refresh(cx));
                cx.notify();
            });
        })
        .detach();
    }
}
