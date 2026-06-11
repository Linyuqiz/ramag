//! 连接新增 / 编辑表单。在 DbClientView 内嵌入，提供「测试连接」+「保存」

use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement, Styled,
    Subscription, Window, div, px,
};
use gpui_component::{
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use ramag_app::{ConnectionService, MongoService, RedisService};
use ramag_domain::entities::{ConnectionColor, ConnectionConfig, ConnectionId, DriverKind};

/// 表单模式：新增 or 编辑
#[derive(Debug, Clone)]
pub enum FormMode {
    Create,
    Edit(ConnectionId),
}

/// 测试结果
#[derive(Debug, Clone)]
pub(super) enum TestState {
    Idle,
    Testing,
    Success,
    Failed(String),
}

/// 表单事件
#[derive(Debug, Clone)]
pub enum FormEvent {
    /// 用户保存成功
    Saved(ConnectionConfig),
    /// 用户取消
    Cancelled,
}

/// driver 元数据 (id, 显示名, 当前可用)。UI 选择器按此顺序从左到右渲染
const DRIVERS: &[(&str, &str, bool)] = &[
    ("mysql", "MySQL", true),
    ("postgres", "PostgreSQL", true),
    ("redis", "Redis", true),
    ("mongodb", "MongoDB", true),
];

/// 连接表单面板
pub struct ConnectionFormPanel {
    service: Arc<ConnectionService>,
    /// Redis 服务（test_connection 时按 driver 路由）；Storage 与 service 共用
    redis_service: Arc<RedisService>,
    /// MongoDB 服务（同上，按 driver 路由）
    mongo_service: Arc<MongoService>,
    pub(super) mode: FormMode,
    /// 当前选中的 driver id（"mysql" / "postgres" / ...）
    pub(super) driver_id: &'static str,
    pub(super) name: Entity<InputState>,
    pub(super) host: Entity<InputState>,
    pub(super) port: Entity<InputState>,
    pub(super) username: Entity<InputState>,
    pub(super) password: Entity<InputState>,
    pub(super) database: Entity<InputState>,
    /// 颜色标签（环境提示）
    color: ConnectionColor,
    pub(super) test_state: TestState,
    pub(super) saving: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<FormEvent> for ConnectionFormPanel {}

impl ConnectionFormPanel {
    pub fn new_create(
        service: Arc<ConnectionService>,
        redis_service: Arc<RedisService>,
        mongo_service: Arc<MongoService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(
            service,
            redis_service,
            mongo_service,
            FormMode::Create,
            None,
            window,
            cx,
        )
    }

    pub fn new_edit(
        service: Arc<ConnectionService>,
        redis_service: Arc<RedisService>,
        mongo_service: Arc<MongoService>,
        existing: ConnectionConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mode = FormMode::Edit(existing.id.clone());
        Self::build(
            service,
            redis_service,
            mongo_service,
            mode,
            Some(existing),
            window,
            cx,
        )
    }

    fn build(
        service: Arc<ConnectionService>,
        redis_service: Arc<RedisService>,
        mongo_service: Arc<MongoService>,
        mode: FormMode,
        prefill: Option<ConnectionConfig>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // 新建：输入框留空，默认值以 placeholder 虚影呈现（保存留空回退，见 ops::validate）
        // 编辑：回填已有连接的真实值
        let is_create = prefill.is_none();
        let p = prefill.unwrap_or_else(|| ConnectionConfig {
            id: ConnectionId::new(),
            name: String::new(),
            driver: DriverKind::Mysql,
            host: String::new(),
            port: 0,
            username: String::new(),
            password: String::new(),
            database: None,
            remark: None,
            color: Default::default(),
        });
        let driver_id = driver_kind_to_id(p.driver);
        let port_text = if is_create {
            String::new()
        } else {
            p.port.to_string()
        };
        // 名称留空时保存即用 Host 作为连接名，虚影同步显示这一默认
        let name_placeholder = if p.host.is_empty() {
            defaults::DEFAULT_HOST.to_string()
        } else {
            p.host.clone()
        };

        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(name_placeholder)
                .default_value(p.name)
        });
        let host = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(defaults::DEFAULT_HOST)
                .default_value(p.host)
        });
        let port = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(defaults::default_port(driver_id).to_string())
                .default_value(port_text)
        });
        let username = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(defaults::username_placeholder(driver_id))
                .default_value(p.username)
        });
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("（留空表示无密码）")
                .default_value(p.password)
        });
        let database = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(defaults::database_placeholder(driver_id))
                .default_value(p.database.unwrap_or_default())
        });

        // host 变化 → 名称虚影跟随（名称始终留给用户输入，不写入真实值）
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(
            &host,
            window,
            |this: &mut Self, _, _e: &InputEvent, window, cx| {
                let host_val = this.host.read(cx).value().trim().to_string();
                let preview = if host_val.is_empty() {
                    defaults::DEFAULT_HOST.to_string()
                } else {
                    host_val
                };
                this.name.update(cx, |state, cx| {
                    state.set_placeholder(preview, window, cx);
                });
            },
        ));

        let initial_color = p.color;

        Self {
            service,
            redis_service,
            mongo_service,
            mode,
            driver_id,
            name,
            host,
            port,
            username,
            password,
            database,
            color: initial_color,
            test_state: TestState::Idle,
            saving: false,
            _subscriptions: subscriptions,
        }
    }
}

/// 在调用方使用：根据 mode 计算 dialog 标题（不显示具体 driver，已由表单内 driver 选择行体现）
pub fn dialog_title(mode: &FormMode) -> &'static str {
    match mode {
        FormMode::Create => "新建连接",
        FormMode::Edit(_) => "编辑连接",
    }
}

impl ConnectionFormPanel {
    /// 公开 mode 给 dialog 标题使用
    pub fn mode(&self) -> &FormMode {
        &self.mode
    }
}

pub(super) fn section_title(text: &str, muted_fg: gpui::Hsla) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap(px(8.0))
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(muted_fg)
                .child(text.to_string()),
        )
        .child(div().flex_1().h(px(1.0)).bg(muted_fg).opacity(0.12))
}

pub(super) fn field_row(label: &str, input: Input) -> impl IntoElement {
    v_flex()
        .gap(px(6.0))
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(label.to_string()),
        )
        .child(div().w_full().child(input))
}

/// DriverKind → driver_id 字符串（用于 UI 选择器内部状态）
fn driver_kind_to_id(kind: DriverKind) -> &'static str {
    match kind {
        DriverKind::Mysql => "mysql",
        DriverKind::Postgres => "postgres",
        DriverKind::Redis => "redis",
        DriverKind::Mongodb => "mongodb",
    }
}

/// driver_id → DriverKind；不可用 / 未来 driver 返回 None
fn id_to_driver_kind(id: &str) -> Option<DriverKind> {
    match id {
        "mysql" => Some(DriverKind::Mysql),
        "postgres" => Some(DriverKind::Postgres),
        "redis" => Some(DriverKind::Redis),
        "mongodb" => Some(DriverKind::Mongodb),
        _ => None,
    }
}

/// ConnectionColor → 实际颜色（连接列表 / Tab Bar / 表单选择器共用）
pub fn color_to_hsla(color: ConnectionColor, theme: &gpui_component::Theme) -> gpui::Hsla {
    use gpui::hsla;
    match color {
        ConnectionColor::None => theme.muted,
        ConnectionColor::Gray => hsla(0.0 / 360.0, 0.0, 0.55, 1.0),
        ConnectionColor::Green => hsla(140.0 / 360.0, 0.55, 0.45, 1.0),
        ConnectionColor::Blue => hsla(210.0 / 360.0, 0.65, 0.55, 1.0),
        ConnectionColor::Yellow => hsla(45.0 / 360.0, 0.85, 0.55, 1.0),
        ConnectionColor::Red => hsla(0.0 / 360.0, 0.7, 0.55, 1.0),
    }
}

// 注：ConnectionFormPanel 没有提供 cx.new 工厂函数，因为 InputState 需要 &mut Window，
// 而 cx.new 的闭包只能拿到 Context。调用方必须在持有 Window 的上下文里直接调用：
//   `cx.new(|cx| ConnectionFormPanel::new_create(svc, window, cx))`

mod defaults;
mod ops;
mod render;
