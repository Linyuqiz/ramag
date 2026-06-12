//! 连接管理页：行点击=打开（emit Selected），行内按钮独立 emit。
//! 搜索按 名称 / host / 用户名 / 数据库 不区分大小写子串匹配

mod render;
mod row;

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{AppContext as _, Context, Entity, EventEmitter, Window};
use gpui_component::input::{InputEvent, InputState};
use ramag_app::{ConnectionService, MongoService, RedisService};
use ramag_domain::entities::{ConnectionConfig, ConnectionId, DriverKind};
use tracing::{debug, error};

pub struct ConnectionListPanel {
    pub(super) service: Arc<ConnectionService>,
    /// Redis 连接的 server_version 走 redis_service
    redis_service: Arc<RedisService>,
    /// MongoDB 连接的 server_version 走 mongo_service
    mongo_service: Arc<MongoService>,
    pub(super) connections: Vec<ConnectionConfig>,
    pub(super) selected: Option<ConnectionId>,
    pub(super) loading: bool,
    pub(super) search: Entity<InputState>,
    /// 小写的搜索关键字
    pub(super) query: String,
    /// 服务端版本缓存。失败连接不入缓存避免重试
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
        mongo_service: Arc<MongoService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx
            .new(|cx| InputState::new(window, cx).placeholder("搜索连接（名称 / host / 用户名）"));

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
            mongo_service,
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
                // refresh 不批量探测版本，避免对未打开连接反复试连不可达主机；
                // 真正 open_session 时由外层调 prefetch_version
            });
        })
        .detach();
    }

    /// 已缓存则跳过；失败仅 debug
    pub fn prefetch_version(&mut self, id: &ConnectionId, cx: &mut Context<Self>) {
        if self.versions.contains_key(id) {
            return;
        }
        let Some(conn) = self.connections.iter().find(|c| &c.id == id).cloned() else {
            return;
        };
        let mysql_svc = self.service.clone();
        let redis_svc = self.redis_service.clone();
        let mongo_svc = self.mongo_service.clone();
        cx.spawn(async move |this, cx| {
            let result = match conn.driver {
                DriverKind::Mysql | DriverKind::Postgres => mysql_svc.server_version(&conn).await,
                DriverKind::Redis => redis_svc.server_version(&conn).await,
                DriverKind::Mongodb => mongo_svc.server_version(&conn).await,
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

    pub fn connections(&self) -> &[ConnectionConfig] {
        &self.connections
    }

    pub(super) fn handle_click(&mut self, conn: ConnectionConfig, cx: &mut Context<Self>) {
        self.selected = Some(conn.id.clone());
        cx.emit(ListEvent::Selected(conn));
        cx.notify();
    }

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
