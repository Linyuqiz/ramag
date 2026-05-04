//! 单行渲染（命名空间或叶子）+ 类型徽标配色
//!
//! 通过 `impl KeyTreePanel` 复用主结构的方法（非 free function），
//! 因为闭包内仍要调用 `panel.select_key` / `panel.toggle_expanded`。

use gpui::{
    ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div, prelude::*, px,
};
use gpui_component::h_flex;
use ramag_domain::entities::RedisType;

use super::tree::VisibleRow;
use super::{INDENT_PX, KeyTreePanel};

impl KeyTreePanel {
    /// 渲染单行（命名空间或叶子）
    ///
    /// `+ use<>`：Rust 2024 默认捕获所有 lifetime，会让返回值绑死在 &self 上，
    /// 与同函数内 `cx.listener(...)` 需要的 `&mut Context<Self>` 借用冲突。
    /// 显式声明不捕获生命周期，确保返回值是 'static 风格
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_node_row(
        &self,
        row: &VisibleRow,
        selected: &Option<String>,
        fg: gpui::Hsla,
        muted_fg: gpui::Hsla,
        row_hover: gpui::Hsla,
        accent: gpui::Hsla,
        theme_bg: gpui::Hsla,
        theme_muted: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let is_namespace = row.is_namespace;
        let is_leaf = row.leaf_type.is_some();
        let is_selected = is_leaf && selected.as_deref() == Some(row.full_path.as_str());

        let row_id = SharedString::from(format!("redis-tree-{}-{}", row.depth, row.full_path));
        let path_for_click = row.full_path.clone();
        let path_for_load = row.full_path.clone();

        // 折叠/展开图标（命名空间专属）
        let chevron: gpui::AnyElement = if is_namespace {
            let glyph = if row.is_expanded { "▼" } else { "▶" };
            div()
                .w(px(12.0))
                .text_xs()
                .text_color(muted_fg)
                .child(glyph)
                .into_any_element()
        } else {
            div().w(px(12.0)).into_any_element()
        };

        // 类型 badge（叶子或同时叶子+命名空间）
        let type_badge: Option<gpui::AnyElement> = row.leaf_type.map(|t| {
            let path = path_for_load.clone();
            div()
                .id(SharedString::from(format!("badge-{}", row.full_path)))
                .text_xs()
                .px(px(5.0))
                .py(px(1.0))
                .rounded(px(3.0))
                .bg(type_color_solid(t, theme_muted))
                .text_color(theme_bg)
                .cursor_pointer()
                .child(t.label())
                // badge 单击：始终加载值（不冒泡到行 toggle）
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.select_key(path.clone(), cx);
                }))
                .into_any_element()
        });

        // 行点击：用一个统一闭包按 is_namespace 分支，避免 if/else 产生不同 closure 类型
        let toggle_mode = is_namespace;
        let on_row_click = cx.listener(move |this, _: &ClickEvent, _, cx| {
            if toggle_mode {
                this.toggle_expanded(path_for_click.clone(), cx);
            } else {
                this.select_key(path_for_click.clone(), cx);
            }
        });

        let label_color = if is_namespace && !is_leaf {
            muted_fg
        } else {
            fg
        };

        // 显式行高 28px：uniform_list 行级虚拟化要求等高
        let mut row_el = h_flex()
            .id(row_id)
            .w_full()
            .h(px(28.0))
            .flex_none()
            .items_center()
            .gap(px(6.0))
            .pl(px(8.0 + row.depth as f32 * INDENT_PX))
            .pr(px(10.0))
            .cursor_pointer()
            .child(chevron)
            .when_some(type_badge, |this, b| this.child(b))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(label_color)
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(row.label.clone()),
            )
            .on_click(on_row_click);

        if is_selected {
            let mut active_bg = accent;
            active_bg.a = 0.18;
            row_el = row_el.bg(active_bg);
        } else {
            row_el = row_el.hover(move |this| this.bg(row_hover));
        }
        row_el
    }
}

/// 不同类型用不同色块（与 RedisInsight / zedis 配色靠拢）
///
/// 接受一个 fallback（None 类型 / theme.muted 等场景）避免依赖完整 theme 引用
fn type_color_solid(kind: RedisType, fallback: gpui::Hsla) -> gpui::Hsla {
    use gpui::hsla;
    match kind {
        RedisType::String => hsla(210.0 / 360.0, 0.6, 0.55, 1.0),
        RedisType::List => hsla(140.0 / 360.0, 0.5, 0.5, 1.0),
        RedisType::Hash => hsla(280.0 / 360.0, 0.55, 0.6, 1.0),
        RedisType::Set => hsla(40.0 / 360.0, 0.85, 0.55, 1.0),
        RedisType::ZSet => hsla(20.0 / 360.0, 0.7, 0.55, 1.0),
        RedisType::Stream => hsla(330.0 / 360.0, 0.55, 0.55, 1.0),
        RedisType::None => fallback,
    }
}
