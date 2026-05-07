//! ConnectionFormPanel Render：driver 选择 + 字段分组 + 测试 / 取消 / 保存

use gpui::{
    ClickEvent, Context, IntoElement, ParentElement, Render, Styled, Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    v_flex,
};

use super::{ConnectionFormPanel, FormMode, TestState, field_row, section_title};

impl Render for ConnectionFormPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // self.render_driver_selector 跨文件调用：mod.rs 内的方法定义在 impl 块（pub(super) fn）
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

        // driver 相关的标签 / 占位
        // PG 协议要求连接时必须绑定具体 database，单独标"必填"以区别 MySQL 的可选
        let is_redis = self.driver_id == "redis";
        let database_label = match self.driver_id {
            "redis" => "DB（0-15）",
            "postgres" => "默认库（必填）",
            _ => "默认库（可选）",
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
