//! 左侧 Activity Bar：纯图标导航。顶部 Home 图标 + 每个工具图标，选中发 NavEvent

use std::sync::Arc;

use gpui::{
    ClickEvent, Context, EventEmitter, IntoElement, ParentElement, Render, SharedString, Styled,
    Window, div, hsla, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use ramag_app::ToolRegistry;

use crate::icons;

#[derive(Debug, Clone, PartialEq)]
pub enum NavTarget {
    Home,
    Tool(String),
}

#[derive(Debug, Clone)]
pub enum NavEvent {
    Navigate(NavTarget),
}

const BAR_WIDTH: f32 = 48.0;
const ITEM_HEIGHT: f32 = 40.0;

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

    /// MySQL/Redis/Postgres 共用 dbclient 入口，driver 在连接表单内选
    fn icon_for_tool(tool_id: &str) -> Icon {
        match tool_id {
            "dbclient" => icons::database(),
            "vcs" => icons::git_branch(),
            "clipboard" => icons::clipboard(),
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

        container = container.child(div().w(px(20.0)).h(px(1.0)).bg(border).my_1());

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

        // 底部：太阳/月亮主题切换（暗色显太阳 → 点击切浅色，反之亦然）+ 设置占位
        container = container.child(div().flex_1());
        let toggle_icon = match crate::theme::current_mode(cx) {
            crate::theme::Mode::Dark => IconName::Sun,
            crate::theme::Mode::Light => IconName::Moon,
        };
        container = container.child(activity_item(
            "theme-toggle",
            Icon::new(toggle_icon),
            false,
            accent,
            transparent,
            |_: &ClickEvent, _, app| {
                // 点击时现取当前模式，避免渲染后主题被系统外观联动改过导致切错方向
                let next = crate::theme::current_mode(app).toggled();
                set_theme(next, app);
            },
        ));
        container = container.child(activity_item(
            "settings",
            Icon::new(IconName::Settings),
            false,
            accent,
            transparent,
            |_: &ClickEvent, _, _| {},
        ));

        container
    }
}

/// 切主题 + 持久化。用户显式选过则 follow_system=false
fn set_theme(mode: crate::theme::Mode, app: &mut gpui::App) {
    if crate::theme::current_mode(app) == mode && !crate::theme::is_following_system(app) {
        return;
    }
    crate::theme::apply_theme(mode, app);
    crate::theme::set_following_system(app, false);
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

/// 选中时左侧 2px accent 竖条
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
        .child(
            div()
                .w(px(2.0))
                .h(px(20.0))
                .bg(if is_selected { accent } else { transparent }),
        )
        .child(
            Button::new(SharedString::from(id.to_string()))
                .ghost()
                .icon(icon)
                .on_click(on_click),
        )
}
