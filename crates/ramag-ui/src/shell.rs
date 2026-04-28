//! Shell：Ramag 主壳
//!
//! 三段式布局：
//! ```text
//! ┌────────────────────────────────────────────────┐
//! │ TitleBar                                       │
//! ├──┬─────────────────────────────────────────────┤
//! │  │                                             │
//! │  │  Tool Content / HomeView                    │
//! │AB│  (sidebar 由各 Tool 自带在内部)             │
//! │  │                                             │
//! └──┴─────────────────────────────────────────────┘
//! ```
//!
//! AB = ActivityBar（左 52px 纯图标）
//!
//! Shell 不知道具体 Tool 视图渲染什么，外部通过 `register_view` 注入。
//! 默认选中是 Home（None），ActivityBar 顶部 ⌂ 高亮。

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    AnyView, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window,
    div, prelude::*,
};
use gpui_component::{ActiveTheme, Root, h_flex, v_flex};
use ramag_app::ToolRegistry;

use crate::activity_bar::{ActivityBar, NavEvent, NavTarget};

/// 主壳视图
pub struct Shell {
    activity_bar: Entity<ActivityBar>,

    /// 工具视图（启动时注入）
    tool_views: HashMap<String, AnyView>,
    /// 首页视图（必有，由外部注入）
    home_view: Option<AnyView>,

    /// 当前激活：None = 首页，Some(tool_id) = 某工具
    selected: Option<String>,

    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(
        registry: Arc<ToolRegistry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let activity_bar = cx.new(|_| ActivityBar::new(registry.clone()));

        let mut subs = Vec::new();
        // 监听 ActivityBar 的 NavEvent
        subs.push(cx.subscribe_in(
            &activity_bar,
            window,
            |this, _, event: &NavEvent, window, cx| match event {
                NavEvent::Navigate(target) => {
                    this.handle_navigate(target.clone(), window, cx);
                }
            },
        ));

        Self {
            activity_bar,
            tool_views: HashMap::new(),
            home_view: None,
            selected: None,
            _subscriptions: subs,
        }
    }

    /// 注入首页视图
    pub fn set_home_view(&mut self, view: AnyView) {
        self.home_view = Some(view);
    }

    /// 注入某个 Tool 的根视图
    pub fn register_tool_view(&mut self, tool_id: impl Into<String>, view: AnyView) {
        self.tool_views.insert(tool_id.into(), view);
    }

    /// 程序内导航到指定目标（不改 ActivityBar UI 状态以外的事）
    pub fn navigate_to(&mut self, target: NavTarget, window: &mut Window, cx: &mut Context<Self>) {
        self.activity_bar
            .update(cx, |bar, cx| bar.set_selected(target.clone(), cx));
        self.handle_navigate(target, window, cx);
    }

    fn handle_navigate(&mut self, target: NavTarget, window: &mut Window, cx: &mut Context<Self>) {
        let new_selected = match target {
            NavTarget::Home => None,
            NavTarget::Tool(id) => Some(id),
        };

        // 标题栏文字置空（保留红绿灯按钮，但不显示 app/page 名称）
        // macOS app 名仍由 dock / 任务栏体现，无需窗口标题再重复
        window.set_window_title("");

        if self.selected != new_selected {
            self.selected = new_selected;
            cx.notify();
        }
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 提前拷出颜色，避免 theme 借用与 cx 可变借用冲突
        let bg_color = cx.theme().background;
        let fg_color = cx.theme().foreground;

        // 决定当前内容视图
        let content_view: Option<AnyView> = match &self.selected {
            None => self.home_view.clone(),
            Some(id) => self.tool_views.get(id).cloned(),
        };

        // gpui-component 的 dialog / notification 浮层（必须由顶层 view 渲染才会生效）
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let notification_layer = Root::render_notification_layer(window, cx);

        // 标题栏由 macOS 原生处理（细，含红绿灯，可双击 zoom）
        // 内容直接从下方开始，无需手动留顶部空间
        v_flex()
            .size_full()
            .bg(bg_color)
            .text_color(fg_color)
            // 主体：左 ActivityBar + 右内容
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.activity_bar.clone())
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .when_some(content_view, |this, view| this.child(view))
                            .when(
                                self.selected.is_some()
                                    && self
                                        .selected
                                        .as_ref()
                                        .and_then(|id| self.tool_views.get(id))
                                        .is_none(),
                                |this| this.child(render_view_missing(cx)),
                            ),
                    ),
            )
            // 浮层：dialog + notification toast
            .children(dialog_layer)
            .children(notification_layer)
    }
}

fn render_view_missing(cx: &Context<Shell>) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(div().text_lg().child("视图未注册"))
        .child(
            div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("请检查 ramag-bin/main.rs 是否调用了 register_tool_view"),
        )
}

