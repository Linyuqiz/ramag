//! 单行连接（整行点击 = 打开；行内编辑/删除独立 emit）
//!
//! driver badge + 名称 + 只读标记 + 版本 / 地址 / 账号固定列对齐 + 编辑/删除按钮。

use gpui::{
    ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div, prelude::*, px,
};
use gpui_component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};
use ramag_domain::entities::{ConnectionConfig, DriverKind};

use super::{ConnectionListPanel, ListEvent};

#[allow(clippy::too_many_arguments)]
pub(super) fn connection_row(
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
    let kind_label = match conn.driver {
        DriverKind::Mysql => "MySQL",
        DriverKind::Postgres => "PostgreSQL",
        DriverKind::Redis => "Redis",
        DriverKind::Mongodb => "MongoDB",
    };

    // driver 配色（一类一色，便于扫一眼连接列表区分）：
    // MySQL 蓝（主题 accent）/ PostgreSQL 紫 / Redis 红 / MongoDB 绿
    let badge_fg: gpui::Hsla = match conn.driver {
        DriverKind::Mysql => accent,
        DriverKind::Postgres => gpui::hsla(265.0 / 360.0, 0.55, 0.55, 1.0),
        DriverKind::Redis => gpui::hsla(0.0, 0.65, 0.55, 1.0),
        DriverKind::Mongodb => gpui::hsla(140.0 / 360.0, 0.55, 0.45, 1.0),
    };
    let mut badge_bg = badge_fg;
    badge_bg.a = 0.12;

    let row_id = SharedString::from(format!("conn-row-{}-{}", idx, conn.id));
    let edit_id = SharedString::from(format!("conn-edit-{}-{}", idx, conn.id));
    let del_id = SharedString::from(format!("conn-del-{}-{}", idx, conn.id));

    let conn_for_open = conn.clone();
    let conn_for_edit = conn.clone();
    let conn_id_for_del = conn.id.clone();
    let is_production = conn.production;

    let host_port = format!("{}:{}", conn.host, conn.port);

    // 名字 = host 时（用户没改默认同步），名字列已显示 host:port，地址列留空避免重复
    let name_collapsed_with_host = conn.name == conn.host;
    let primary_label = if name_collapsed_with_host {
        host_port.clone()
    } else {
        conn.name.clone()
    };
    let address_text = if name_collapsed_with_host {
        String::new()
    } else {
        host_port
    };

    // 账号：空段省略，避免 Redis 这类无 user / db 的连接显示无意义的「— @ —」
    let account_text = {
        let user = conn.username.trim();
        let db = conn.database.as_deref().map(str::trim).unwrap_or("");
        match (user.is_empty(), db.is_empty()) {
            (false, false) => format!("{user} @ {db}"),
            (false, true) => user.to_string(),
            (true, false) => db.to_string(),
            (true, true) => String::new(),
        }
    };

    let version_text = version
        .map(|v| format!("{kind_label} {v}"))
        .unwrap_or_default();

    // 固定宽度的次要信息列：内容为空也占位，保证各行整列对齐
    let secondary_col = move |w: f32, text: String| {
        div()
            .flex_none()
            .w(px(w))
            .text_xs()
            .text_color(muted_fg)
            .overflow_hidden()
            .text_ellipsis()
            .child(text)
    };

    let danger = gpui::hsla(0.0, 0.7, 0.55, 1.0);
    let mut prod_bg = danger;
    prod_bg.a = 0.15;

    let mut row = h_flex()
        .id(row_id)
        .w_full()
        .items_center()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(8.0))
        .border_b_1()
        .border_color(border)
        .cursor_pointer()
        .hover(move |this| this.bg(hover_bg))
        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.handle_click(conn_for_open.clone(), cx);
        }))
        // 类型 badge（固定宽度，一类一色）
        .child(
            div().flex_none().w(px(76.0)).flex().justify_center().child(
                div()
                    .px(px(8.0))
                    .py(px(2.0))
                    .rounded(px(4.0))
                    .text_xs()
                    .text_color(badge_fg)
                    .bg(badge_bg)
                    .child(kind_label),
            ),
        )
        // 名称（占主空间）
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(fg)
                .overflow_hidden()
                .text_ellipsis()
                .child(primary_label),
        )
        // 只读徽章槽（固定宽，生产连接显示红色「只读」，否则空白占位 → 整列对齐）
        .child(div().flex_none().w(px(44.0)).flex().justify_center().when(
            is_production,
            move |slot| {
                slot.child(
                    div()
                        .px(px(6.0))
                        .py(px(1.0))
                        .rounded(px(4.0))
                        .text_xs()
                        .text_color(danger)
                        .bg(prod_bg)
                        .child("只读"),
                )
            },
        ))
        // 版本（固定列，空占位）
        .child(secondary_col(120.0, version_text))
        // 地址 host:port（固定列，名称已含则空占位）
        .child(secondary_col(150.0, address_text))
        // 账号 user @ db（固定列，空占位）
        .child(secondary_col(150.0, account_text))
        // 操作按钮（编辑 / 删除）：图标按钮 + tooltip
        // mouse_down 拦截避免点击事件冒泡到父行触发"打开连接"
        .child(
            h_flex()
                .flex_none()
                .gap(px(4.0))
                .w(px(72.0))
                .justify_end()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    Button::new(edit_id)
                        .ghost()
                        .small()
                        .icon(ramag_ui::icons::pencil())
                        .tooltip("编辑连接")
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(ListEvent::RequestEdit(conn_for_edit.clone()));
                        })),
                )
                .child(
                    Button::new(del_id)
                        .ghost()
                        .small()
                        .icon(ramag_ui::icons::trash())
                        .tooltip("删除连接")
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(ListEvent::RequestDelete(conn_id_for_del.clone()));
                        })),
                ),
        );

    if is_selected {
        let mut sel_bg = accent;
        sel_bg.a = 0.06;
        row = row.bg(sel_bg);
    }

    row
}
