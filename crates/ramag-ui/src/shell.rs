//! 主壳：原生 TitleBar + 左 ActivityBar（52px）+ 右 Tool/HomeView。视图由外部 register_tool_view 注入

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    AnyView, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window,
    div, prelude::*,
};
use gpui_component::{ActiveTheme, Root, h_flex, v_flex};
use ramag_app::ToolRegistry;

use crate::activity_bar::{ActivityBar, NavEvent, NavTarget};

pub struct Shell {
    activity_bar: Entity<ActivityBar>,
    tool_views: HashMap<String, AnyView>,
    home_view: Option<AnyView>,
    /// None=首页，Some(tool_id)=某工具
    selected: Option<String>,

    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(registry: Arc<ToolRegistry>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let activity_bar = cx.new(|_| ActivityBar::new(registry.clone()));

        let mut subs = Vec::new();
        subs.push(cx.subscribe_in(
            &activity_bar,
            window,
            |this, _, event: &NavEvent, window, cx| match event {
                NavEvent::Navigate(target) => {
                    this.handle_navigate(target.clone(), window, cx);
                }
            },
        ));
        // 跟随系统主题：用户显式选过则忽略
        subs.push(cx.observe_window_appearance(window, |_this, window, cx| {
            crate::theme::on_system_appearance_changed(window.appearance(), cx);
        }));

        Self {
            activity_bar,
            tool_views: HashMap::new(),
            home_view: None,
            selected: None,
            _subscriptions: subs,
        }
    }

    pub fn set_home_view(&mut self, view: AnyView) {
        self.home_view = Some(view);
    }

    pub fn register_tool_view(&mut self, tool_id: impl Into<String>, view: AnyView) {
        self.tool_views.insert(tool_id.into(), view);
    }

    /// 程序内导航
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

        // 标题置空保留红绿灯，dock 已显示 app 名
        window.set_window_title("");

        if self.selected != new_selected {
            self.selected = new_selected;
            cx.notify();
        }
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 先拷颜色避开 theme 借用与 cx 可变借用冲突
        let bg_color = cx.theme().background;
        let fg_color = cx.theme().foreground;

        let content_view: Option<AnyView> = match &self.selected {
            None => self.home_view.clone(),
            Some(id) => self.tool_views.get(id).cloned(),
        };

        // dialog / notification 浮层须由顶层 view 渲染
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let notification_layer = Root::render_notification_layer(window, cx);

        v_flex()
            .size_full()
            .bg(bg_color)
            .text_color(fg_color)
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
