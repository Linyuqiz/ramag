//! 单行连接（整行点击 = 打开；行内编辑/删除独立 emit）
//!
//! 一类一色 driver badge + host:port + user@db + 编辑/删除图标按钮。

use gpui::{
    ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Sizable as _,
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
        DriverKind::Postgres => "PostgreSQL",
        DriverKind::Redis => "Redis",
    };

    // driver 配色（一类一色，便于扫一眼连接列表区分）：
    // - MySQL：蓝（沿用主题 accent，保持品牌主线）
    // - PostgreSQL：紫（贴近 PG 海豚品牌色）
    // - Redis：红（贴近 Redis 官方红）
    let badge_fg: gpui::Hsla = match conn.driver {
        DriverKind::Mysql => accent,
        DriverKind::Postgres => gpui::hsla(265.0 / 360.0, 0.55, 0.55, 1.0),
        DriverKind::Redis => gpui::hsla(0.0, 0.65, 0.55, 1.0),
    };
    let mut badge_bg = badge_fg;
    badge_bg.a = 0.12;

    let row_id = SharedString::from(format!("conn-row-{}-{}", idx, conn.id));
    let edit_id = SharedString::from(format!("conn-edit-{}-{}", idx, conn.id));
    let del_id = SharedString::from(format!("conn-del-{}-{}", idx, conn.id));

    let conn_for_open = conn.clone();
    let conn_for_edit = conn.clone();
    let conn_id_for_del = conn.id.clone();

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

    // 名字 = host 时（用户没改默认同步），名字列合并显示 host:port，
    // 避免右侧 host:port 列重复同样的信息
    let name_collapsed_with_host = conn.name == conn.host;
    let primary_label = if name_collapsed_with_host {
        host_port.clone()
    } else {
        conn.name.clone()
    };

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
        // 类型 badge（固定宽度，整齐对齐；按 driver 一类一色）
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
                .child(primary_label),
        )
        // 服务端版本（仅在已成功探测到时显示；未拉到则该列不占空间）
        .when_some(version, |row, v| {
            row.child(
                div()
                    .flex_none()
                    .w(px(130.0))
                    .text_xs()
                    .text_color(muted_fg)
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(format!("{kind_label} {v}")),
            )
        })
        // host:port（仅在名字未与 host 合并时显示，避免重复）
        .when(!name_collapsed_with_host, |row| {
            row.child(
                div()
                    .flex_none()
                    .w(px(180.0))
                    .text_xs()
                    .text_color(muted_fg)
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(host_port),
            )
        })
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
        // 操作按钮（编辑 / 删除）：图标按钮 + tooltip，与项目其他面板一致
        // mouse_down 拦截避免点击事件冒泡到父行触发"打开连接"
        .child(
            h_flex()
                .flex_none()
                .gap(px(4.0))
                .w(px(80.0))
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
