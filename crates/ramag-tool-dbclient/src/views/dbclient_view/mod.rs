//! DB Client 根视图：多连接 Tab，顶部连接 tab bar + 中心 session / picker

mod dialogs;
mod render;

use std::sync::Arc;

use gpui::{
    AnyView, App, AppContext as _, Context, Entity, Point, ScrollHandle, Subscription, Window, px,
};
use ramag_app::{ConnectionService, MongoService, RedisService};
use ramag_domain::entities::{ConnectionConfig, DriverKind};

use ramag_tool_mongodb::MongoSessionPanel;
use ramag_tool_redis::RedisSessionPanel;

use crate::views::connection_list::{ConnectionListPanel, ListEvent};
use crate::views::connection_session::ConnectionSession;

/// 当前主区显示什么
pub(super) enum CenterMode {
    /// 显示某个 Session（active_session 索引）
    Session,
    /// 显示连接管理（保存的连接列表 + 新建）
    ConnectionPicker,
}

/// SQL 类（MySQL / Postgres / 未来 SQLite）走 ConnectionSession；Redis 走 RedisSessionPanel；MongoDB 走 MongoSessionPanel
pub(super) enum SessionEntity {
    Sql(Entity<ConnectionSession>),
    Redis(Entity<RedisSessionPanel>),
    Mongo(Entity<MongoSessionPanel>),
}

impl SessionEntity {
    pub(super) fn config<'a>(&'a self, cx: &'a App) -> &'a ConnectionConfig {
        match self {
            SessionEntity::Sql(e) => e.read(cx).config(),
            SessionEntity::Redis(e) => e.read(cx).config(),
            SessionEntity::Mongo(e) => e.read(cx).config(),
        }
    }
    pub(super) fn title<'a>(&'a self, cx: &'a App) -> &'a str {
        match self {
            SessionEntity::Sql(e) => e.read(cx).title(),
            SessionEntity::Redis(e) => e.read(cx).title(),
            SessionEntity::Mongo(e) => e.read(cx).title(),
        }
    }
    /// 数据库类型副标签（Tab Bar 副标题）。Sql 变体走 ConnectionSession 自身的 kind_label
    pub(super) fn kind_label<'a>(&'a self, cx: &'a App) -> &'static str {
        match self {
            SessionEntity::Sql(e) => e.read(cx).kind_label(),
            SessionEntity::Redis(_) => "Redis",
            SessionEntity::Mongo(_) => "MongoDB",
        }
    }
    pub(super) fn to_any_view(&self) -> AnyView {
        match self {
            SessionEntity::Sql(e) => e.clone().into(),
            SessionEntity::Redis(e) => e.clone().into(),
            SessionEntity::Mongo(e) => e.clone().into(),
        }
    }
    /// Tab 被激活时触发：各 session 内部按「树为空才补拉」决定是否真正加载，
    /// 既保证「打开就能用」，连接放久后切回也会重新请求（驱动层在取连接时自愈死连接）
    pub(super) fn ensure_loaded(&self, cx: &mut App) {
        match self {
            SessionEntity::Sql(e) => e.update(cx, |s, cx| s.ensure_loaded(cx)),
            SessionEntity::Redis(e) => e.update(cx, |s, cx| s.ensure_loaded(cx)),
            SessionEntity::Mongo(e) => e.update(cx, |s, cx| s.ensure_loaded(cx)),
        }
    }
}

pub struct DbClientView {
    pub(super) service: Arc<ConnectionService>,
    pub(super) redis_service: Arc<RedisService>,
    pub(super) mongo_service: Arc<MongoService>,
    /// 已打开的连接会话（含 MySQL + Redis + MongoDB）
    pub(super) sessions: Vec<SessionEntity>,
    /// 当前激活的 session 索引
    pub(super) active_session: Option<usize>,
    /// 中央显示模式
    pub(super) center: CenterMode,
    /// 连接管理面板（始终持有，按需展示）
    pub(super) picker: Entity<ConnectionListPanel>,
    /// 顶部连接 tab bar 横向滚动句柄：连接多到溢出时新开后滚到末尾
    pub(super) sessions_scroll: ScrollHandle,
    pub(super) _subscriptions: Vec<Subscription>,
}

impl DbClientView {
    pub fn new(
        service: Arc<ConnectionService>,
        redis_service: Arc<RedisService>,
        mongo_service: Arc<MongoService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let picker = cx.new(|cx| {
            ConnectionListPanel::new(
                service.clone(),
                redis_service.clone(),
                mongo_service.clone(),
                window,
                cx,
            )
        });

        let subs = vec![cx.subscribe_in(&picker, window, Self::on_picker_event)];

        Self {
            service,
            redis_service,
            mongo_service,
            sessions: Vec::new(),
            active_session: None,
            // 启动时显示连接管理（用户挑选打开哪个）
            center: CenterMode::ConnectionPicker,
            picker,
            sessions_scroll: ScrollHandle::new(),
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
            .position(|s| s.config(cx).id == config.id)
        {
            self.active_session = Some(idx);
            self.center = CenterMode::Session;
            // 重新激活已打开的连接：树为空则补拉（含首次加载失败后的重试）
            self.sessions[idx].ensure_loaded(cx);
            cx.notify();
            return;
        }

        // 在 config 被 move 进 session 之前先抓 id 用于版本探测
        let conn_id = config.id.clone();
        // 按 driver dispatch：SQL 类（MySQL/Postgres）走 ConnectionSession；Redis 走 RedisSessionPanel；MongoDB 走 MongoSessionPanel
        let new_session = match config.driver {
            DriverKind::Mysql | DriverKind::Postgres => {
                let svc = self.service.clone();
                let entity = cx.new(|cx| ConnectionSession::new(config, svc, window, cx));
                SessionEntity::Sql(entity)
            }
            DriverKind::Redis => {
                let svc = self.redis_service.clone();
                let entity = cx.new(|cx| RedisSessionPanel::new(config, svc, window, cx));
                SessionEntity::Redis(entity)
            }
            DriverKind::Mongodb => {
                let svc = self.mongo_service.clone();
                let entity = cx.new(|cx| MongoSessionPanel::new(config, svc, window, cx));
                SessionEntity::Mongo(entity)
            }
        };
        self.sessions.push(new_session);
        self.active_session = Some(self.sessions.len() - 1);
        self.center = CenterMode::Session;
        // tab 多溢出时让新连接 tab 滚入视图（GPUI 自动 clamp 到 max_offset）
        self.sessions_scroll
            .set_offset(Point::new(px(-99999.0), px(0.0)));
        // 用户主动打开后才异步探测版本（不打开的连接不会去建池/试连）
        self.picker
            .update(cx, |p, cx| p.prefetch_version(&conn_id, cx));
        cx.notify();
    }

    /// 关闭某个 Session Tab
    pub(super) fn close_session(&mut self, idx: usize, cx: &mut Context<Self>) {
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

    pub(super) fn select_session(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.sessions.len() {
            self.active_session = Some(idx);
            self.center = CenterMode::Session;
            // 切到该 Tab：树为空则补拉（含首次加载失败后的重试）
            self.sessions[idx].ensure_loaded(cx);
            cx.notify();
        }
    }

    /// 切到"打开连接"面板
    pub(super) fn show_picker(&mut self, cx: &mut Context<Self>) {
        self.center = CenterMode::ConnectionPicker;
        // 刷新一下列表
        self.picker.update(cx, |p, cx| p.refresh(cx));
        cx.notify();
    }
}
