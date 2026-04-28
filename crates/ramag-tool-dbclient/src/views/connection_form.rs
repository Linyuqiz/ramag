//! 连接表单（新增 / 编辑）
//!
//! 弹出（或嵌入）的表单：name / host / port / user / password / database。
//! 提供"测试连接"和"保存"两个按钮。
//!
//! Stage 2 简化：使用嵌入式表单（在 DbClientView 中作为子面板显示），
//! 不引入完整 Modal 系统。Stage 3 可改为 Modal。

use std::sync::Arc;

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, SharedString,
    Styled, Subscription, Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use ramag_app::{ConnectionService, RedisService};
use ramag_domain::entities::{ConnectionColor, ConnectionConfig, ConnectionId, DriverKind};
use tracing::{error, info};

/// 表单模式：新增 or 编辑
#[derive(Debug, Clone)]
pub enum FormMode {
    Create,
    Edit(ConnectionId),
}

/// 测试结果
#[derive(Debug, Clone)]
enum TestState {
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

/// 数据库 driver 元数据（id / 显示名 / 是否当前可用）
///
/// 顺序固定：UI 选择器从左到右按此渲染。
/// MongoDB / SQLite 暂不在路线图，先从选择器中移除避免误导
const DRIVERS: &[(&str, &str, bool)] = &[
    ("mysql", "MySQL", true),
    ("postgres", "PostgreSQL", false),
    ("redis", "Redis", true),
];

/// 连接表单面板
pub struct ConnectionFormPanel {
    service: Arc<ConnectionService>,
    /// Redis 服务（test_connection 时按 driver 路由）；Storage 与 service 共用
    redis_service: Arc<RedisService>,
    mode: FormMode,
    /// 当前选中的 driver id（"mysql" / "postgres" / ...）
    /// 当前只有 mysql 可选；其他显示但不可点
    driver_id: &'static str,
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    database: Entity<InputState>,
    /// 颜色标签（环境提示）
    color: ConnectionColor,
    test_state: TestState,
    saving: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<FormEvent> for ConnectionFormPanel {}

impl ConnectionFormPanel {
    pub fn new_create(
        service: Arc<ConnectionService>,
        redis_service: Arc<RedisService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(service, redis_service, FormMode::Create, None, window, cx)
    }

    pub fn new_edit(
        service: Arc<ConnectionService>,
        redis_service: Arc<RedisService>,
        existing: ConnectionConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mode = FormMode::Edit(existing.id.clone());
        Self::build(service, redis_service, mode, Some(existing), window, cx)
    }

    fn build(
        service: Arc<ConnectionService>,
        redis_service: Arc<RedisService>,
        mode: FormMode,
        prefill: Option<ConnectionConfig>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let p = prefill.unwrap_or_else(|| ConnectionConfig {
            id: ConnectionId::new(),
            name: String::new(),
            driver: DriverKind::Mysql,
            host: "127.0.0.1".to_string(),
            port: 3306,
            username: "root".to_string(),
            password: String::new(),
            database: None,
            remark: None,
            color: Default::default(),
        });

        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("如 prod-mysql")
                .default_value(p.name)
        });
        let host = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("127.0.0.1")
                .default_value(p.host)
        });
        let port = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("3306")
                .default_value(p.port.to_string())
        });
        let username = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("root")
                .default_value(p.username)
        });
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("（留空表示无密码）")
                .default_value(p.password)
        });
        let database = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("可选")
                .default_value(p.database.unwrap_or_default())
        });

        // host 变化 + name 当前为空 → 自动同步 name = host
        // 一旦 name 非空（用户开始输入），不再覆盖；用户清空 name 后又会重新跟随
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(
            &host,
            window,
            |this: &mut Self, _, _e: &InputEvent, window, cx| {
                if !this.name.read(cx).value().is_empty() {
                    return;
                }
                let host_val = this.host.read(cx).value().to_string();
                if host_val.is_empty() {
                    return;
                }
                this.name.update(cx, |state, cx| {
                    state.set_value(host_val, window, cx);
                });
            },
        ));

        let initial_color = p.color;
        let driver_id = driver_kind_to_id(p.driver);

        Self {
            service,
            redis_service,
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

    /// 切换 driver（仅可用 driver 才会通过 UI 触发）
    ///
    /// 端口字段联动：仅当用户没改动过端口（仍是另一 driver 的默认值）时才自动切换
    /// - mysql ↔ redis：3306 ↔ 6379
    fn set_driver(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        if self.driver_id == id {
            return;
        }
        let cur_port = self.port.read(cx).value().to_string();
        let new_port: Option<&'static str> = match (self.driver_id, id) {
            ("mysql", "redis") if cur_port == "3306" || cur_port.is_empty() => Some("6379"),
            ("redis", "mysql") if cur_port == "6379" || cur_port.is_empty() => Some("3306"),
            _ => None,
        };
        if let Some(np) = new_port {
            self.port
                .update(cx, |state, cx| state.set_value(np, window, cx));
        }
        self.driver_id = id;
        cx.notify();
    }

    /// 校验表单并返回 ConnectionConfig；任意必填项缺失返回中文错误描述
    fn validate(&self, cx: &gpui::App) -> Result<ConnectionConfig, String> {
        let name = self.name.read(cx).value().trim().to_string();
        let host = self.host.read(cx).value().trim().to_string();
        let port_str = self.port.read(cx).value().trim().to_string();
        let username = self.username.read(cx).value().trim().to_string();
        let password = self.password.read(cx).value().to_string();
        let database = {
            let v = self.database.read(cx).value().trim().to_string();
            if v.is_empty() { None } else { Some(v) }
        };

        if name.is_empty() {
            return Err("请填写连接名称".into());
        }
        if host.is_empty() {
            return Err("请填写 Host".into());
        }
        let port: u16 = port_str
            .parse()
            .map_err(|_| "Port 必须是 1 - 65535 的数字".to_string())?;
        if port == 0 {
            return Err("Port 必须是 1 - 65535".into());
        }

        let driver =
            id_to_driver_kind(self.driver_id).ok_or_else(|| "请选择数据库类型".to_string())?;

        // 用户名：MySQL 必填；Redis 可空（老版无 ACL 时用空用户名）
        if matches!(driver, DriverKind::Mysql) && username.is_empty() {
            return Err("请填写用户名".into());
        }
        // Redis 的 DB 字段限制 0-255 数字
        if matches!(driver, DriverKind::Redis) {
            if let Some(ref s) = database {
                s.parse::<u8>()
                    .map_err(|_| "DB 必须是 0 - 255 的数字（默认 Redis 上限 0-15）".to_string())?;
            }
        }
        let id = match &self.mode {
            FormMode::Create => ConnectionId::new(),
            FormMode::Edit(id) => id.clone(),
        };

        Ok(ConnectionConfig {
            id,
            name,
            driver,
            host,
            port,
            username,
            password,
            database,
            remark: None,
            color: self.color,
        })
    }

    /// 渲染 driver 选择器：5 个按钮横排，仅可用 driver 可点
    fn render_driver_selector(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let accent = theme.accent;
        let border = theme.border;
        let secondary_bg = theme.secondary;

        let mut accent_tint = accent;
        accent_tint.a = 0.10;
        let mut accent_border = accent;
        accent_border.a = 0.55;

        let mut row = h_flex().w_full().items_center().gap(px(8.0));
        for &(id, name, available) in DRIVERS {
            let is_selected = self.driver_id == id;
            let btn_id = SharedString::from(format!("driver-btn-{id}"));

            // 5 个按钮等分宽度（flex_1 + min_w_0），避免文字长的 PostgreSQL 撑破布局
            let mut btn = h_flex()
                .id(btn_id)
                .flex_1()
                .min_w_0()
                .items_center()
                .justify_center()
                .gap(px(6.0))
                .px(px(8.0))
                .py(px(7.0))
                .rounded_md()
                .border_1()
                .text_sm()
                .child(name.to_string());

            if is_selected {
                // 选中态：accent 描边 + 浅 accent 底
                btn = btn
                    .bg(accent_tint)
                    .border_color(accent_border)
                    .text_color(accent);
            } else if available {
                // 可点击未选中
                btn = btn
                    .bg(secondary_bg)
                    .border_color(border)
                    .text_color(fg)
                    .cursor_pointer()
                    .hover(move |this| this.border_color(accent_border))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.set_driver(id, window, cx);
                    }));
            } else {
                // 禁用：dim、不可点
                btn = btn
                    .bg(secondary_bg)
                    .border_color(border)
                    .text_color(muted_fg)
                    .opacity(0.45);
            }

            row = row.child(btn);
        }

        v_flex()
            .gap(px(8.0))
            .child(section_title("数据库类型", muted_fg))
            .child(row)
    }

    fn handle_test(&mut self, cx: &mut Context<Self>) {
        let config = match self.validate(cx) {
            Ok(c) => c,
            Err(e) => {
                self.test_state = TestState::Failed(e);
                cx.notify();
                return;
            }
        };
        self.test_state = TestState::Testing;
        cx.notify();

        // 按 driver 走对应的 service.test：MySQL → ConnectionService；Redis → RedisService
        let mysql_svc = self.service.clone();
        let redis_svc = self.redis_service.clone();
        cx.spawn(async move |this, cx| {
            let result = match config.driver {
                DriverKind::Mysql => mysql_svc.test(&config).await,
                DriverKind::Redis => redis_svc.test(&config).await,
            };
            let _ = this.update(cx, |this, cx| {
                this.test_state = match result {
                    Ok(_) => {
                        info!("test_connection ok");
                        TestState::Success
                    }
                    Err(e) => {
                        error!(error = %e, "test_connection failed");
                        TestState::Failed(e.to_string())
                    }
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn handle_save(&mut self, cx: &mut Context<Self>) {
        let config = match self.validate(cx) {
            Ok(c) => c,
            Err(e) => {
                self.test_state = TestState::Failed(e);
                cx.notify();
                return;
            }
        };
        self.saving = true;
        cx.notify();

        let svc = self.service.clone();
        cx.spawn(async move |this, cx| {
            let result = svc.save(&config).await;
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                match result {
                    Ok(_) => {
                        info!(name = %config.name, "connection saved");
                        cx.emit(FormEvent::Saved(config));
                    }
                    Err(e) => {
                        error!(error = %e, "save connection failed");
                        this.test_state = TestState::Failed(format!("保存失败：{e}"));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn handle_cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(FormEvent::Cancelled);
    }
}

impl Render for ConnectionFormPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let border = theme.border;

        let test_msg = match &self.test_state {
            TestState::Idle => None,
            TestState::Testing => Some(("测试中...".to_string(), muted_fg)),
            TestState::Success => Some(("✓ 连接成功".to_string(), gpui::green())),
            TestState::Failed(msg) => Some((msg.clone(), gpui::red())),
        };

        // 内容（不带 dialog 标题/边框，dialog 系统提供）：
        // driver 选择器（仅新建可见）→ 字段分组 → 底部按钮区
        // 注：dialog 自身有 16px padding，这里只补少量上下间距
        let driver_selector: Option<gpui::AnyElement> = matches!(self.mode, FormMode::Create)
            .then(|| self.render_driver_selector(cx).into_any_element());

        // driver 相关的标签 / 占位（Redis 与 SQL 类形态略有差异）
        let is_redis = self.driver_id == "redis";
        let database_label = if is_redis {
            "DB（0-15）"
        } else {
            "默认库（可选）"
        };
        let username_label = if is_redis {
            "用户名（ACL，可选）"
        } else {
            "用户名"
        };

        v_flex()
            .w_full()
            .gap(px(18.0))
            .pt(px(4.0))
            .pb(px(4.0))
            // —— 数据库类型（仅新建时显示，编辑模式 driver 不可变更）——
            .children(driver_selector)
            // —— 连接信息 ——
            .child(
                v_flex()
                    .gap(px(12.0))
                    .child(section_title("连接信息", muted_fg))
                    .child(field_row("名称", Input::new(&self.name)))
                    .child(
                        h_flex()
                            .w_full()
                            .gap(px(12.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(field_row("Host", Input::new(&self.host))),
                            )
                            .child(
                                div()
                                    .w(px(110.0))
                                    .child(field_row("Port", Input::new(&self.port))),
                            ),
                    )
                    .child(field_row(database_label, Input::new(&self.database))),
            )
            // —— 认证 ——
            .child(
                v_flex()
                    .gap(px(12.0))
                    .child(section_title("认证", muted_fg))
                    .child(field_row(username_label, Input::new(&self.username)))
                    .child(field_row("密码", Input::new(&self.password))),
            )
            // —— 分隔 + 按钮区 ——
            .child(div().h(px(1.0)).bg(border).my(px(2.0)))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap(px(12.0))
                            .child(Button::new("test").small().label("测试连接").on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.handle_test(cx);
                                }),
                            ))
                            .when_some(test_msg, |this, (msg, color)| {
                                this.child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::NORMAL)
                                        .text_color(color)
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .child(msg),
                                )
                            }),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(8.0))
                            .flex_none()
                            .child(
                                Button::new("cancel")
                                    .ghost()
                                    .small()
                                    .label("取消")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.handle_cancel(cx);
                                    })),
                            )
                            .child(
                                Button::new("save")
                                    .primary()
                                    .small()
                                    .label(if self.saving {
                                        "保存中..."
                                    } else {
                                        "保存"
                                    })
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        if !this.saving {
                                            this.handle_save(cx);
                                        }
                                    })),
                            ),
                    ),
            )
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

fn section_title(text: &str, muted_fg: gpui::Hsla) -> impl IntoElement {
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

fn field_row(label: &str, input: Input) -> impl IntoElement {
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
///
/// Redis 不在 DBClient 工具的表单选择器内呈现（Redis 走独立的 Redis 工具），
/// 此处仍要覆盖以满足穷举 match；id 选用 "redis" 与未来 Redis 表单约定保持一致
fn driver_kind_to_id(kind: DriverKind) -> &'static str {
    match kind {
        DriverKind::Mysql => "mysql",
        DriverKind::Redis => "redis",
    }
}

/// driver_id → DriverKind；不可用 / 未来 driver 返回 None
fn id_to_driver_kind(id: &str) -> Option<DriverKind> {
    match id {
        "mysql" => Some(DriverKind::Mysql),
        "redis" => Some(DriverKind::Redis),
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
