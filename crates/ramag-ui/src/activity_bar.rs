//! 左侧 Activity Bar：52px 宽的纯图标导航
//!
//! 顶部固定一个 ⌂ Home 图标，下面跟着每个工具的图标。
//! 选中态高亮（左侧 2px 竖条 + 图标变亮），发出 NavEvent 通知 Shell 切换内容。

use std::sync::Arc;

use gpui::{
    ClickEvent, Context, EventEmitter, IntoElement, ParentElement, Render, SharedString, Styled,
    Window, div, hsla, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use ramag_app::ToolRegistry;

use crate::icons;

/// 当前选中状态
#[derive(Debug, Clone, PartialEq)]
pub enum NavTarget {
    /// 首页
    Home,
    /// 某个工具
    Tool(String),
}

/// 导航事件
#[derive(Debug, Clone)]
pub enum NavEvent {
    Navigate(NavTarget),
}

const BAR_WIDTH: f32 = 48.0;
const ITEM_HEIGHT: f32 = 40.0;

/// Activity Bar 组件
pub struct ActivityBar {
    registry: Arc<ToolRegistry>,
    selected: NavTarget,
}

impl EventEmitter<NavEvent> for ActivityBar {}

impl ActivityBar {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self {
            registry,
            selected: NavTarget::Home,
        }
    }

    pub fn set_selected(&mut self, target: NavTarget, cx: &mut Context<Self>) {
        if self.selected != target {
            self.selected = target;
            cx.notify();
        }
    }

    fn navigate(&mut self, target: NavTarget, cx: &mut Context<Self>) {
        self.selected = target.clone();
        cx.emit(NavEvent::Navigate(target));
        cx.notify();
    }

    /// 根据工具 id 选一个图标：
    /// - `dbclient` 走 ramag 自带 svg（database.svg）
    /// - 其他先用上游 IconName 兜底
    fn icon_for_tool(tool_id: &str) -> Icon {
        match tool_id {
            "dbclient" => icons::database(),
            "jsonfmt" => Icon::new(IconName::File),
            "url" => Icon::new(IconName::Globe),
            "hash" => Icon::new(IconName::MemoryStick),
            _ => Icon::new(IconName::Inbox),
        }
    }
}

impl Render for ActivityBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let tools = self.registry.list();
        let selected = self.selected.clone();

        let accent = theme.accent;
        let sidebar_bg = theme.sidebar;
        let border = theme.border;
        let transparent = hsla(0.0, 0.0, 0.0, 0.0);

        let mut container = v_flex()
            .w(px(BAR_WIDTH))
            .h_full()
            .flex_none()
            .bg(sidebar_bg)
            .border_r_1()
            .border_color(border)
            .py_2()
            .gap_1()
            .items_center();

        // 首页（lucide house 风格，由 ramag 自有 svg 提供）
        let is_home_selected = matches!(selected, NavTarget::Home);
        container = container.child(activity_item(
            "home",
            icons::home(),
            is_home_selected,
            accent,
            transparent,
            cx.listener(|this, _: &ClickEvent, _, cx| {
                this.navigate(NavTarget::Home, cx);
            }),
        ));

        // 分隔线
        container = container.child(div().w(px(20.0)).h(px(1.0)).bg(border).my_1());

        // 各工具图标
        for tool in tools.iter() {
            let id = tool.meta().id.clone();
            let id_for_click = id.clone();
            let is_selected = matches!(&selected, NavTarget::Tool(s) if s == &id);
            let icon = Self::icon_for_tool(&id);

            container = container.child(activity_item(
                &format!("tool-{id}"),
                icon,
                is_selected,
                accent,
                transparent,
                cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.navigate(NavTarget::Tool(id_for_click.clone()), cx);
                }),
            ));
        }

        // 底部设置按钮：占用剩余空间下推，固定在最底部
        container = container.child(div().flex_1());
        let current_mode = crate::theme::current_mode(cx);
        let is_dark = matches!(current_mode, crate::theme::Mode::Dark);
        // 当前选中项前缀加 ✓ 标记；未选中两空格占位保持文字对齐。
        // 不用 PopupMenuItem.checked()：上游会把整行染成 accent 蓝，与暗色面板不搭。
        let label_light = if is_dark { "  浅色" } else { "✓ 浅色" };
        let label_dark = if is_dark { "✓ 暗色" } else { "  暗色" };
        container = container.child(
            h_flex()
                .w(px(BAR_WIDTH))
                .h(px(ITEM_HEIGHT))
                .items_center()
                .justify_center()
                .child(div().w(px(2.0)).h(px(20.0)).bg(transparent))
                .child(
                    Button::new("settings")
                        .ghost()
                        .icon(IconName::Settings)
                        // BottomLeft anchor：菜单弹按钮上方而非右侧
                        // 否则子菜单"主题"展开会盖住底部的设置按钮本身
                        .dropdown_menu_with_anchor(gpui::Anchor::BottomLeft, move |menu, window, cx| {
                            // 保留 Sun / Moon 图标：让用户一眼能识别两个选项
                            // ✓ 选中标记由 label 前缀承担（避免 .checked() 把整行染成 accent 蓝）
                            menu.submenu("主题", window, cx, move |sub, _, _| {
                                sub.item(
                                    PopupMenuItem::new(label_light)
                                        .icon(IconName::Sun)
                                        .on_click(|_, _, app| set_theme(crate::theme::Mode::Light, app)),
                                )
                                .item(
                                    PopupMenuItem::new(label_dark)
                                        .icon(IconName::Moon)
                                        .on_click(|_, _, app| set_theme(crate::theme::Mode::Dark, app)),
                                )
                            })
                        }),
                ),
        );

        container
    }
}

/// 切到指定主题并持久化偏好
fn set_theme(mode: crate::theme::Mode, app: &mut gpui::App) {
    if crate::theme::current_mode(app) == mode {
        return;
    }
    crate::theme::apply_theme(mode, app);
    app.refresh_windows();
    if let Some(storage) = crate::theme::storage_from_cx(app) {
        let value = match mode {
            crate::theme::Mode::Dark => "dark".to_string(),
            crate::theme::Mode::Light => "light".to_string(),
        };
        app.background_executor()
            .spawn(async move {
                if let Err(e) = storage.set_preference("theme_mode", &value).await {
                    tracing::warn!(error = %e, "failed to persist theme");
                }
            })
            .detach();
    }
}

/// 单条 Activity Bar 图标项
///
/// 选中时左侧显示 2px 高亮竖条
fn activity_item(
    id: &str,
    icon: Icon,
    is_selected: bool,
    accent: gpui::Hsla,
    transparent: gpui::Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    h_flex()
        .w(px(BAR_WIDTH))
        .h(px(ITEM_HEIGHT))
        .items_center()
        .justify_center()
        // 左侧选中高亮竖条
        .child(
            div()
                .w(px(2.0))
                .h(px(20.0))
                .bg(if is_selected { accent } else { transparent }),
        )
        // 图标按钮（默认尺寸；Button 自身宽度由内边距决定）
        .child(
            Button::new(SharedString::from(id.to_string()))
                .ghost()
                .icon(icon)
                .on_click(on_click),
        )
}
