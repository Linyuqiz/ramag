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

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render,
    SharedString, Styled, Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    v_flex,
};
use ramag_app::{ConnectionService, RedisService};
use ramag_domain::entities::{ConnectionConfig, ConnectionId, DriverKind};
use tracing::{debug, error};

pub struct ConnectionListPanel {
    service: Arc<ConnectionService>,
    /// Redis 服务：拉取 Redis 连接的 server_version 走它（与 MySQL 服务并列）
    redis_service: Arc<RedisService>,
    connections: Vec<ConnectionConfig>,
    selected: Option<ConnectionId>,
    loading: bool,
    /// 搜索输入框（持有以便订阅 Change 事件）
    search: Entity<InputState>,
    /// 当前搜索关键字（小写，用于过滤；空表示不过滤）
    query: String,
    /// 服务端版本缓存：key=ConnectionId，value="8.0.32" / "7.2.4" 等
    /// refresh 后串行后台 fetch；失败的连接不缓存（避免反复重试）
    versions: HashMap<ConnectionId, String>,
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
                // list 加载完后串行拉版本，已缓存的跳过；失败不卡 UI
                this.fetch_versions(cx);
            });
        })
        .detach();
    }

    /// 串行后台拉每个连接的服务端版本（已缓存跳过；失败仅 debug 日志）
    /// 串行避免一次性建多个池连导致目标库瞬时压力（每条都走 driver 的连接池）
    ///
    /// 按 driver 路由：MySQL → ConnectionService（sqlx）；Redis → RedisService（redis-rs）
    fn fetch_versions(&mut self, cx: &mut Context<Self>) {
        let conns: Vec<ConnectionConfig> = self.connections.clone();
        let mysql_svc = self.service.clone();
        let redis_svc = self.redis_service.clone();
        cx.spawn(async move |this, cx| {
            for conn in conns {
                let cached = this
                    .update(cx, |this, _| this.versions.contains_key(&conn.id))
                    .unwrap_or(true);
                if cached {
                    continue;
                }
                let result = match conn.driver {
                    DriverKind::Mysql => mysql_svc.server_version(&conn).await,
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

    fn handle_click(&mut self, conn: ConnectionConfig, cx: &mut Context<Self>) {
        self.selected = Some(conn.id.clone());
        cx.emit(ListEvent::Selected(conn));
        cx.notify();
    }

    /// 按当前关键字过滤连接列表
    fn filtered(&self) -> Vec<ConnectionConfig> {
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

impl Render for ConnectionListPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let accent = theme.accent;
        let border = theme.border;
        let row_hover = theme.muted;
        let bg = theme.background;

        let total = self.connections.len();
        let loading = self.loading;
        let visible = self.filtered();
        let visible_count = visible.len();
        let selected = self.selected.clone();

        // 内容统一限制最大宽度 1080px 居中，避免大屏摊得太开
        // 头部和列表行用同一个容器宽度，左右对齐整齐
        const CONTENT_MAX_W: f32 = 1080.0;

        // ===== Header =====
        // 极简布局：左侧搜索框（max 360px）+ 右侧"新建连接"（outline + small，更克制）
        let header_inner = h_flex()
            .w_full()
            .items_center()
            .gap(px(16.0))
            .child(
                div().flex_1().min_w_0().child(
                    div().max_w(px(360.0)).child(
                        Input::new(&self.search)
                            .small()
                            .cleanable(true)
                            .prefix(Icon::new(IconName::Search).small().text_color(muted_fg)),
                    ),
                ),
            )
            .child(
                Button::new("add-connection")
                    .outline()
                    .small()
                    .icon(IconName::Plus)
                    .label("新建连接")
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        cx.emit(ListEvent::RequestNew);
                    })),
            );

        // 顶部和 tab bar 之间留出呼吸空间（pt 比 pb 略大）
        let header = h_flex()
            .w_full()
            .justify_center()
            .px(px(24.0))
            .pt(px(22.0))
            .pb(px(16.0))
            .border_b_1()
            .border_color(border)
            .child(div().w_full().max_w(px(CONTENT_MAX_W)).child(header_inner));

        // ===== Body =====
        let body: AnyElement = if loading {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(div().text_sm().text_color(muted_fg).child("加载中..."))
                .into_any_element()
        } else if total == 0 {
            empty_state(border, muted_fg, fg, accent, cx).into_any_element()
        } else if visible_count == 0 {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .child(
                    div()
                        .text_sm()
                        .text_color(fg)
                        .child(format!("没有匹配「{}」的连接", self.query)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted_fg)
                        .child("尝试修改关键字或清空搜索"),
                )
                .into_any_element()
        } else {
            let mut rows: Vec<AnyElement> = Vec::with_capacity(visible_count);
            for (idx, conn) in visible.into_iter().enumerate() {
                let is_selected = selected.as_ref() == Some(&conn.id);
                let version = self.versions.get(&conn.id).cloned();
                rows.push(
                    connection_row(
                        idx,
                        conn,
                        is_selected,
                        version,
                        border,
                        row_hover,
                        accent,
                        fg,
                        muted_fg,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            v_flex()
                .size_full()
                .overflow_y_scrollbar()
                .child(
                    h_flex()
                        .w_full()
                        .justify_center()
                        .px(px(24.0))
                        .py(px(10.0))
                        .child(v_flex().w_full().max_w(px(CONTENT_MAX_W)).children(rows)),
                )
                .into_any_element()
        };

        v_flex().size_full().bg(bg).child(header).child(body)
    }
}

/// 单行连接（整行点击 = 打开；行内编辑/删除独立 emit）
#[allow(clippy::too_many_arguments)]
fn connection_row(
    idx: usize,
    conn: ConnectionConfig,
    is_selected: bool,
    // 服务端版本（None = 还没拉到 / 拉失败）
    version: Option<String>,
    border: gpui::Hsla,
    hover_bg: gpui::Hsla,
    accent: gpui::Hsla,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    cx: &mut Context<ConnectionListPanel>,
) -> impl IntoElement {
    use ramag_domain::entities::ConnectionColor;

    // 仅当用户给连接打了颜色标签（区分生产/开发等环境）时才渲染圆点
    // 没设颜色时显示无意义的灰圆点会干扰视觉，干脆隐藏
    let theme_for_color = cx.theme();
    let status_dot: Option<gpui::Hsla> = if conn.color != ConnectionColor::None {
        Some(crate::views::connection_form::color_to_hsla(
            conn.color,
            theme_for_color,
        ))
    } else {
        None
    };
    let _ = muted_fg; // 圆点逻辑不再用 muted_fg；保留参数避免改签名

    let kind_label = match conn.driver {
        DriverKind::Mysql => "MySQL",
        DriverKind::Redis => "Redis",
    };

    let row_id = SharedString::from(format!("conn-row-{}-{}", idx, conn.id));
    let edit_id = SharedString::from(format!("conn-edit-{}-{}", idx, conn.id));
    let del_id = SharedString::from(format!("conn-del-{}-{}", idx, conn.id));

    let conn_for_open = conn.clone();
    let conn_for_edit = conn.clone();
    let conn_id_for_del = conn.id.clone();

    let mut accent_tint = accent;
    accent_tint.a = 0.10;

    let host_port = format!("{}:{}", conn.host, conn.port);
    // 用户名空（如 Redis 老版无 ACL）显示 "—"，与 db 字段空时一致，避免 "@ 0" 的视觉割裂
    let username_text = if conn.username.is_empty() {
        "—".to_string()
    } else {
        conn.username.clone()
    };
    let user_db = format!(
        "{} @ {}",
        username_text,
        conn.database.clone().unwrap_or_else(|| "—".into())
    );

    let mut row = h_flex()
        .id(row_id)
        .w_full()
        .items_center()
        .gap(px(14.0))
        .px(px(14.0))
        .py(px(12.0))
        .border_b_1()
        .border_color(border)
        .cursor_pointer()
        .hover(move |this| this.bg(hover_bg))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.handle_click(conn_for_open.clone(), cx);
        }))
        // 状态点：仅在用户设置了 ConnectionColor（环境标签）时显示
        .when_some(status_dot, |row, color| {
            row.child(
                div()
                    .flex_none()
                    .w(px(10.0))
                    .h(px(10.0))
                    .rounded_full()
                    .bg(color),
            )
        })
        // 类型 badge（固定宽度，整齐对齐）
        .child(
            div().flex_none().w(px(56.0)).flex().justify_center().child(
                div()
                    .px(px(8.0))
                    .py(px(2.0))
                    .rounded(px(4.0))
                    .text_xs()
                    .text_color(accent)
                    .bg(accent_tint)
                    .child(kind_label),
            ),
        )
        // 名称（最重要，占主空间）
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(fg)
                .overflow_hidden()
                .text_ellipsis()
                .child(conn.name.clone()),
        )
        // 服务端版本（独立列，弱视觉；未拉到时占位空字符串保持列宽对齐）
        .child(
            div()
                .flex_none()
                .w(px(130.0))
                .text_xs()
                .text_color(muted_fg)
                .overflow_hidden()
                .text_ellipsis()
                .child(match version {
                    Some(v) => format!("{kind_label} {v}"),
                    None => String::new(),
                }),
        )
        // host:port（固定宽度）
        .child(
            div()
                .flex_none()
                .w(px(180.0))
                .text_xs()
                .text_color(muted_fg)
                .overflow_hidden()
                .text_ellipsis()
                .child(host_port),
        )
        // user @ db（固定宽度）
        .child(
            div()
                .flex_none()
                .w(px(160.0))
                .text_xs()
                .text_color(muted_fg)
                .overflow_hidden()
                .text_ellipsis()
                .child(user_db),
        )
        // 操作按钮（编辑 / 删除）
        // 不用 Button：fork 里 ButtonCustomVariant.foreground 是 dead field
        // （text_color 实际用 colors.color），无法做"透明底 + 蓝色文字"的轻量按钮。
        // 直接 div + on_click 做可点击文本，accent 蓝色 = 可点击，无下划线。
        // mouse_down 拦截避免点击事件冒泡到父行触发"打开连接"。
        .child(
            h_flex()
                .flex_none()
                .gap(px(12.0))
                .w(px(80.0))
                .justify_end()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .id(edit_id)
                        .text_xs()
                        .text_color(accent)
                        .cursor_pointer()
                        .hover(|this| this.opacity(0.7))
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(ListEvent::RequestEdit(conn_for_edit.clone()));
                        }))
                        .child("编辑"),
                )
                .child(
                    div()
                        .id(del_id)
                        .text_xs()
                        .text_color(accent)
                        .cursor_pointer()
                        .hover(|this| this.opacity(0.7))
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(ListEvent::RequestDelete(conn_id_for_del.clone()));
                        }))
                        .child("删除"),
                ),
        );

    if is_selected {
        let mut sel_bg = accent;
        sel_bg.a = 0.06;
        row = row.bg(sel_bg);
    }

    row
}

/// 空状态：一个大引导块，主按钮"新建连接"
fn empty_state(
    border: gpui::Hsla,
    muted_fg: gpui::Hsla,
    fg: gpui::Hsla,
    accent: gpui::Hsla,
    cx: &mut Context<ConnectionListPanel>,
) -> impl IntoElement {
    let mut tinted_accent = accent;
    tinted_accent.a = 0.12;

    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap(px(20.0))
        .child(
            div()
                .w(px(64.0))
                .h(px(64.0))
                .rounded(px(14.0))
                .bg(tinted_accent)
                .flex()
                .items_center()
                .justify_center()
                .child(ramag_ui::icons::database().text_color(accent)),
        )
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(fg)
                .child("还没有连接"),
        )
        .child(
            div()
                .text_sm()
                .text_color(muted_fg)
                .child("点击下方按钮创建第一个数据库连接"),
        )
        .child(
            Button::new("empty-add")
                .primary()
                .icon(IconName::Plus)
                .label("新建连接")
                .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                    cx.emit(ListEvent::RequestNew);
                })),
        )
        .pb(px(64.0))
        .pt(px(64.0))
        .mx(px(40.0))
        .border_1()
        .border_color(border)
        .rounded_lg()
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
