//! ConnectionForm 状态 + 异步：driver 切换 / 校验 / 测试 / 保存。render_driver_selector 也在这里

use gpui::{
    ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, Window, prelude::*, px,
};
use gpui_component::{ActiveTheme, h_flex, v_flex};
use ramag_domain::entities::{ConnectionConfig, ConnectionId, DriverKind};
use tracing::{error, info};

use super::{
    ConnectionFormPanel, DRIVERS, FormEvent, FormMode, TestState, id_to_driver_kind, section_title,
};

impl ConnectionFormPanel {
    /// 端口仍是某 driver 默认值（mysql=3306 / postgres=5432 / redis=6379）时才自动切换
    pub(super) fn set_driver(
        &mut self,
        id: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.driver_id == id {
            return;
        }
        let cur_port = self.port.read(cx).value().to_string();
        let is_default_for = |port: &str, driver: &str| -> bool {
            matches!(
                (driver, port),
                ("mysql", "3306") | ("postgres", "5432") | ("redis", "6379") | ("mongodb", "27017")
            )
        };
        // 用户没改过端口（保持当前 driver 默认）才自动切换到新 driver 默认
        let new_port: Option<&'static str> =
            if cur_port.is_empty() || is_default_for(&cur_port, self.driver_id) {
                match id {
                    "mysql" => Some("3306"),
                    "postgres" => Some("5432"),
                    "redis" => Some("6379"),
                    "mongodb" => Some("27017"),
                    _ => None,
                }
            } else {
                None
            };
        if let Some(np) = new_port {
            self.port
                .update(cx, |state, cx| state.set_value(np, window, cx));
        }
        self.driver_id = id;
        cx.notify();
    }

    /// 校验表单并返回 ConnectionConfig；任意必填项缺失返回中文错误描述
    pub(super) fn validate(&self, cx: &gpui::App) -> Result<ConnectionConfig, String> {
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

        // 用户名：MySQL/Postgres 必填；Redis 可空（老版无 ACL 时用空用户名）
        if matches!(driver, DriverKind::Mysql | DriverKind::Postgres) && username.is_empty() {
            return Err("请填写用户名".into());
        }
        // Redis 的 DB 字段限制 0-255 数字
        if matches!(driver, DriverKind::Redis)
            && let Some(ref s) = database
        {
            s.parse::<u8>()
                .map_err(|_| "DB 必须是 0 - 255 的数字（默认 Redis 上限 0-15）".to_string())?;
        }
        // Postgres 必须连接具体 database（不能不指定）
        if matches!(driver, DriverKind::Postgres) && database.is_none() {
            return Err("PostgreSQL 必须填写默认库".into());
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
    pub(super) fn render_driver_selector(&self, cx: &mut Context<Self>) -> impl IntoElement {
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

    pub(super) fn handle_test(&mut self, cx: &mut Context<Self>) {
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

        // 按 driver 走对应的 service.test：SQL 类（MySQL/Postgres）→ ConnectionService；Redis → RedisService；MongoDB → MongoService
        let sql_svc = self.service.clone();
        let redis_svc = self.redis_service.clone();
        let mongo_svc = self.mongo_service.clone();
        cx.spawn(async move |this, cx| {
            let result = match config.driver {
                DriverKind::Mysql | DriverKind::Postgres => sql_svc.test(&config).await,
                DriverKind::Redis => redis_svc.test(&config).await,
                DriverKind::Mongodb => mongo_svc.test(&config).await,
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

    pub(super) fn handle_save(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn handle_cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(FormEvent::Cancelled);
    }
}
