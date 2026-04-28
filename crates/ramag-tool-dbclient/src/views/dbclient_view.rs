//! DbClientView：DB Client 工具的根视图（多连接 Tab 版）
//!
//! 布局：
//! ```text
//! ┌────────────────────────────────────────────────────────┐
//! │ [Conn-A ✕] [Conn-B ✕]              + 打开连接          │ ← 顶部连接 Tab Bar
//! ├────────────────────────────────────────────────────────┤
//! │                                                        │
//! │  当前 Session 内容（左 Tree + 右 QueryPanel）           │
//! │  或：连接管理面板（保存的连接 + 新建按钮）               │
//! │                                                        │
//! └────────────────────────────────────────────────────────┘
//! ```

use std::sync::Arc;

use gpui::{
    AnyView, App, ClickEvent, Context, Entity, IntoElement, ParentElement, Render, SharedString,
    Styled, Subscription, Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use ramag_app::ConnectionService;
use ramag_domain::entities::{ConnectionConfig, ConnectionId};
use tracing::{error, info};

use gpui_component::WindowExt as _;

use crate::views::connection_form::{self, ConnectionFormPanel, FormEvent};
use crate::views::connection_list::{ConnectionListPanel, ListEvent};
use crate::views::connection_session::ConnectionSession;

/// 当前主区显示什么
enum CenterMode {
    /// 显示某个 Session（active_session 索引）
    Session,
    /// 显示连接管理（保存的连接列表 + 新建）
    ConnectionPicker,
}

pub struct DbClientView {
    service: Arc<ConnectionService>,
    /// 已打开的连接会话
    sessions: Vec<Entity<ConnectionSession>>,
    /// 当前激活的 session 索引
    active_session: Option<usize>,
    /// 中央显示模式
    center: CenterMode,
    /// 连接管理面板（始终持有，按需展示）
    picker: Entity<ConnectionListPanel>,
    _subscriptions: Vec<Subscription>,
}

impl DbClientView {
    pub fn new(
        service: Arc<ConnectionService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let picker = cx.new(|cx| ConnectionListPanel::new(service.clone(), window, cx));

        let mut subs = Vec::new();
        subs.push(cx.subscribe_in(&picker, window, Self::on_picker_event));

        Self {
            service,
            sessions: Vec::new(),
            active_session: None,
            // 启动时显示连接管理（用户挑选打开哪个）
            center: CenterMode::ConnectionPicker,
            picker,
            _subscriptions: subs,
        }
    }

    fn on_picker_event(
        &mut self,
        _list: &Entity<ConnectionListPanel>,
        event: &ListEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ListEvent::Selected(conn) => {
                // 选中已保存连接 → 打开为新 Session
                self.open_session(conn.clone(), window, cx);
            }
            ListEvent::RequestNew => {
                self.open_form_create(window, cx);
            }
            ListEvent::RequestEdit(conn) => {
                self.open_form_edit(conn.clone(), window, cx);
            }
            ListEvent::RequestDelete(id) => {
                self.confirm_delete(id.clone(), window, cx);
            }
        }
    }

    /// 打开一个连接作为新 Session（如果已开就切到那个 Tab）
    fn open_session(
        &mut self,
        config: ConnectionConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 已开过的话直接切过去
        if let Some(idx) = self
            .sessions
            .iter()
            .position(|s| s.read(cx).config().id == config.id)
        {
            self.active_session = Some(idx);
            self.center = CenterMode::Session;
            cx.notify();
            return;
        }

        let svc = self.service.clone();
        let session = cx.new(|cx| ConnectionSession::new(config, svc, window, cx));
        self.sessions.push(session);
        self.active_session = Some(self.sessions.len() - 1);
        self.center = CenterMode::Session;
        cx.notify();
    }

    /// 关闭某个 Session Tab
    fn close_session(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.sessions.len() {
            return;
        }
        self.sessions.remove(idx);
        // 调整 active
        if self.sessions.is_empty() {
            self.active_session = None;
            self.center = CenterMode::ConnectionPicker;
        } else if let Some(active) = self.active_session {
            if active == idx {
                // 关闭的就是当前激活：切到前一个或 0
                self.active_session = Some(idx.saturating_sub(1).min(self.sessions.len() - 1));
            } else if active > idx {
                // 关闭的在前面：索引减 1
                self.active_session = Some(active - 1);
            }
        }
        cx.notify();
    }

    fn select_session(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.sessions.len() {
            self.active_session = Some(idx);
            self.center = CenterMode::Session;
            cx.notify();
        }
    }

    /// 切到"打开连接"面板
    fn show_picker(&mut self, cx: &mut Context<Self>) {
        self.center = CenterMode::ConnectionPicker;
        // 刷新一下列表
        self.picker.update(cx, |p, cx| p.refresh(cx));
        cx.notify();
    }

    fn open_form_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let svc = self.service.clone();
        let form = cx.new(|cx| ConnectionFormPanel::new_create(svc, window, cx));
        self.subscribe_form_and_open_dialog(form, window, cx);
    }

    fn open_form_edit(
        &mut self,
        conn: ConnectionConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let svc = self.service.clone();
        let form = cx.new(|cx| ConnectionFormPanel::new_edit(svc, conn, window, cx));
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
            FormEvent::Saved(_conn) => {
                info!("connection saved, refreshing picker");
                window.close_dialog(cx);
                self.picker.update(cx, |p, cx| p.refresh(cx));
                cx.notify();
            }
            FormEvent::Cancelled => {
                window.close_dialog(cx);
            }
        }
    }

    /// 弹出删除确认对话框；用户点「删除」后才真正执行 handle_delete
    fn confirm_delete(
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
        let id_for_ok = id.clone();
        let conn_name_for_dialog = conn_name.clone();

        window.open_dialog(cx, move |dialog, _, _| {
            let id = id_for_ok.clone();
            let view = view.clone();
            let desc = format!(
                "确定要删除连接「{conn_name_for_dialog}」吗？此操作不可撤销。"
            );

            let cancel_btn = Button::new("alert-cancel")
                .ghost()
                .small()
                .label("取消")
                .on_click(|_: &ClickEvent, window, app| {
                    window.close_dialog(app);
                });

            let ok_btn = Button::new("alert-delete")
                .danger()
                .small()
                .label("删除")
                .on_click({
                    let id = id.clone();
                    let view = view.clone();
                    move |_: &ClickEvent, window, app| {
                        view.update(app, |this, cx| {
                            this.handle_delete(id.clone(), cx);
                        });
                        window.close_dialog(app);
                    }
                });

            dialog
                .title("删除连接？")
                // 默认是视口高度的 1/10（顶端贴边），改成约 1/4 让对话框更靠中间
                .margin_top(px(180.0))
                .content(move |content, _, cx| {
                    let muted_fg = cx.theme().muted_foreground;
                    content.child(
                        div()
                            .py(px(4.0))
                            .text_sm()
                            .text_color(muted_fg)
                            .child(desc.clone()),
                    )
                })
                .footer(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_end()
                        .gap(px(8.0))
                        .child(cancel_btn)
                        .child(ok_btn),
                )
        });
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
                    .filter(|(_, s)| s.read(cx).config().id == id_for_async)
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

impl Render for DbClientView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let border = theme.border;
        let secondary_bg = theme.secondary;
        let muted_bg = theme.muted;
        let accent = theme.accent;
        let bg = theme.background;

        let active = self.active_session;

        // ===== 顶部连接 Tab Bar =====
        // 元组：(idx, 连接名, 数据库类型, 是否选中, 颜色标签)
        let session_titles: Vec<(
            usize,
            String,
            &'static str,
            bool,
            ramag_domain::entities::ConnectionColor,
        )> = self
            .sessions
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let s_read = s.read(cx);
                (
                    i,
                    s_read.title().to_string(),
                    s_read.kind_label(),
                    Some(i) == active,
                    s_read.config().color,
                )
            })
            .collect::<Vec<_>>();

        let on_picker_active = matches!(self.center, CenterMode::ConnectionPicker);

        let mut tab_bar = h_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(border)
            .bg(secondary_bg);

        // ===== 第一个固定 tab：数据源管理 =====
        let picker_btn_active = on_picker_active;
        let mut picker_tab = h_flex()
            .id("picker-tab")
            .items_center()
            .gap_2()
            .px_3()
            .py(px(7.0))
            .border_r_1()
            .border_color(border)
            .cursor_pointer()
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.show_picker(cx);
            }))
            .child(
                ramag_ui::icons::database()
                    .small()
                    .text_color(if picker_btn_active { fg } else { muted_fg }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(if picker_btn_active { fg } else { muted_fg })
                    .child("数据源管理"),
            );

        if picker_btn_active {
            let mut active_bg = accent;
            active_bg.a = 0.15;
            picker_tab = picker_tab.bg(active_bg);
        } else {
            picker_tab = picker_tab.hover(move |this| this.bg(muted_bg));
        }
        tab_bar = tab_bar.child(picker_tab);

        // ===== 右侧 session tabs：连接多了横向可滚动（不挤压 picker tab）=====
        let mut session_strip = h_flex()
            .id("conn-tabs-scroll")
            .flex_1()
            .min_w_0()
            .overflow_x_scroll();

        for (idx, title, kind_label, is_active, color_tag) in session_titles {
            let tab_id = SharedString::from(format!("conn-tab-{idx}"));
            let close_id = SharedString::from(format!("conn-tab-close-{idx}"));

            // Tab 状态点：用连接 color 标签优先，未设时回退绿色
            use ramag_domain::entities::ConnectionColor;
            let dot_color = if color_tag != ConnectionColor::None {
                crate::views::connection_form::color_to_hsla(color_tag, cx.theme())
            } else {
                gpui::hsla(120.0 / 360.0, 0.5, 0.5, 1.0)
            };

            let mut tab = h_flex()
                .id(tab_id)
                .flex_none()
                .items_center()
                .gap_2()
                .px_3()
                .py(px(7.0))
                .border_r_1()
                .border_color(border)
                .cursor_pointer()
                .child(
                    div()
                        .w(px(8.0))
                        .h(px(8.0))
                        .rounded_full()
                        .bg(dot_color),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(if is_active { fg } else { muted_fg })
                        .child(title.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted_fg)
                        .child(kind_label),
                )
                .child(
                    Button::new(close_id)
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.close_session(idx, cx);
                        })),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.select_session(idx, cx);
                }));

            if is_active && !on_picker_active {
                let mut active_bg = accent;
                active_bg.a = 0.15;
                tab = tab.bg(active_bg);
            } else {
                tab = tab.hover(move |this| this.bg(muted_bg));
            }

            session_strip = session_strip.child(tab);
        }

        tab_bar = tab_bar.child(session_strip);

        // ===== 中心内容 =====
        let center_view: AnyView = match &self.center {
            CenterMode::Session => match active.and_then(|i| self.sessions.get(i)) {
                Some(s) => s.clone().into(),
                None => self.picker.clone().into(),
            },
            CenterMode::ConnectionPicker => self.picker.clone().into(),
        };

        v_flex()
            .size_full()
            .bg(bg)
            .text_color(fg)
            .child(tab_bar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(center_view),
            )
    }
}

/// 工厂：在 App 上下文创建 DbClientView 并返回 AnyView
pub fn create_dbclient_view(
    service: Arc<ConnectionService>,
    window: &mut Window,
    cx: &mut App,
) -> AnyView {
    let view = cx.new(|cx| DbClientView::new(service, window, cx));
    view.into()
}
