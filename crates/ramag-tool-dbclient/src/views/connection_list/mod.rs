//! 连接管理页（列表版）
//!
//! 布局：
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │ 共 N 个连接 · MySQL   [🔍 搜索连接...]      [+ 新建连接]        │
//! ├─────────────────────────────────────────────────────────────────┤
//! │ ● [MySQL]  midas-dev    10.0.17.38:3306   root @ —    编辑 删除 │
//! │ ● [MySQL]  local        127.0.0.1:3306    root @ —    编辑 删除 │
//! │ ...                                                             │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! 整行点击 = 打开连接（emit `Selected`）；行内编辑/删除按钮独立 emit。
//! 搜索关键字会按 名称 / host / 用户名 / 数据库 做不区分大小写的子串匹配。
//!
//! 模块拆分：
//! - 本文件：state + new + 方法 + ListEvent
//! - `render`：impl Render + 空状态
//! - `row`：单行连接渲染（200 行的 driver badge / host:port / 操作按钮）

mod render;
mod row;

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{AppContext as _, Context, Entity, EventEmitter, Window};
use gpui_component::input::{InputEvent, InputState};
use ramag_app::{ConnectionService, RedisService};
use ramag_domain::entities::{ConnectionConfig, ConnectionId, DriverKind};
use tracing::{debug, error};

pub struct ConnectionListPanel {
    pub(super) service: Arc<ConnectionService>,
    /// Redis 服务：拉取 Redis 连接的 server_version 走它（与 MySQL 服务并列）
    redis_service: Arc<RedisService>,
    pub(super) connections: Vec<ConnectionConfig>,
    pub(super) selected: Option<ConnectionId>,
    pub(super) loading: bool,
    /// 搜索输入框（持有以便订阅 Change 事件）
    pub(super) search: Entity<InputState>,
    /// 当前搜索关键字（小写，用于过滤；空表示不过滤）
    pub(super) query: String,
    /// 服务端版本缓存：key=ConnectionId，value="8.0.32" / "7.2.4" 等
    /// refresh 后串行后台 fetch；失败的连接不缓存（避免反复重试）
    pub(super) versions: HashMap<ConnectionId, String>,
    _subscriptions: Vec<gpui::Subscription>,
}

#[derive(Debug, Clone)]
pub enum ListEvent {
    Selected(ConnectionConfig),
    RequestNew,
    RequestEdit(ConnectionConfig),
    RequestDelete(ConnectionId),
}

impl EventEmitter<ListEvent> for ConnectionListPanel {}

impl ConnectionListPanel {
    pub fn new(
        service: Arc<ConnectionService>,
        redis_service: Arc<RedisService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx
            .new(|cx| InputState::new(window, cx).placeholder("搜索连接（名称 / host / 用户名）"));

        // 订阅搜索框变化 → 同步 query 并刷新
        let mut subs = Vec::new();
        subs.push(cx.subscribe_in(
            &search,
            window,
            |this: &mut Self, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query = this.search.read(cx).value().trim().to_lowercase();
                    cx.notify();
                }
            },
        ));

        let mut this = Self {
            service,
            redis_service,
            connections: Vec::new(),
            selected: None,
            loading: true,
            search,
            query: String::new(),
            versions: HashMap::new(),
            _subscriptions: subs,
        };
        this.refresh(cx);
        this
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        let svc = self.service.clone();
        cx.spawn(async move |this, cx| {
            let result = svc.list().await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(list) => this.connections = list,
                    Err(e) => {
                        error!(error = %e, "list connections failed");
                        this.connections = Vec::new();
                    }
                }
                cx.notify();
                // 不再在 refresh 时批量探测版本：未打开的连接保持沉默，避免反复试连不可达主机
                // 真正打开（open_session）时由外层显式调 prefetch_version 探测一次
            });
        })
        .detach();
    }

    /// 仅探测单条连接的服务端版本（已缓存则跳过；失败仅 debug 日志）
    ///
    /// 由 dbclient_view 在用户主动打开连接成功后调用，避免对未打开的连接建池
    pub fn prefetch_version(&mut self, id: &ConnectionId, cx: &mut Context<Self>) {
        if self.versions.contains_key(id) {
            return;
        }
        let Some(conn) = self.connections.iter().find(|c| &c.id == id).cloned() else {
            return;
        };
        let mysql_svc = self.service.clone();
        let redis_svc = self.redis_service.clone();
        cx.spawn(async move |this, cx| {
            let result = match conn.driver {
                DriverKind::Mysql | DriverKind::Postgres => mysql_svc.server_version(&conn).await,
                DriverKind::Redis => redis_svc.server_version(&conn).await,
            };
            match result {
                Ok(v) => {
                    let _ = this.update(cx, |this, cx| {
                        this.versions.insert(conn.id.clone(), v);
                        cx.notify();
                    });
                }
                Err(e) => {
                    debug!(error = %e, conn = %conn.name, "fetch server version failed");
                }
            }
        })
        .detach();
    }

    pub fn set_selected(&mut self, id: Option<ConnectionId>, cx: &mut Context<Self>) {
        self.selected = id;
        cx.notify();
    }

    pub fn selected(&self) -> Option<&ConnectionId> {
        self.selected.as_ref()
    }

    /// 公开当前已加载的连接列表（用于外层查找名称等元数据）
    pub fn connections(&self) -> &[ConnectionConfig] {
        &self.connections
    }

    pub(super) fn handle_click(&mut self, conn: ConnectionConfig, cx: &mut Context<Self>) {
        self.selected = Some(conn.id.clone());
        cx.emit(ListEvent::Selected(conn));
        cx.notify();
    }

    /// 按当前关键字过滤连接列表
    pub(super) fn filtered(&self) -> Vec<ConnectionConfig> {
        if self.query.is_empty() {
            return self.connections.clone();
        }
        let q = &self.query;
        self.connections
            .iter()
            .filter(|c| {
                c.name.to_lowercase().contains(q)
                    || c.host.to_lowercase().contains(q)
                    || c.username.to_lowercase().contains(q)
                    || c.database
                        .as_deref()
                        .map(|d| d.to_lowercase().contains(q))
                        .unwrap_or(false)
            })
            .cloned()
            .collect()
    }
}

/// 工厂（注：调用方需要持有 `&mut Window`）
pub fn create(
    service: Arc<ConnectionService>,
    redis_service: Arc<RedisService>,
    window: &mut Window,
    cx: &mut gpui::App,
) -> Entity<ConnectionListPanel> {
    cx.new(|cx| ConnectionListPanel::new(service, redis_service, window, cx))
}
