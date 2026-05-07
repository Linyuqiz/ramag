//! 交互式 Rebase 计划编辑器：todo 列表（action dropdown / 上移 / 下移）+ 取消 / 执行

use gpui::{
    AnyElement, App, ClickEvent, Context, InteractiveElement, IntoElement, ParentElement,
    SharedString, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    scroll::ScrollableElement as _,
    v_flex,
};
use ramag_domain::entities::RebaseAction;

use super::vcs_view::VcsView;

impl VcsView {
    /// 交互式 Rebase 计划编辑器主入口（IDE 布局 render_main_area 路由调用）
    pub(super) fn render_rebase_plan(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let border = theme.border;
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let bg = theme.background;
        let busy = self.busy;
        let onto = self.rebase_plan_onto.clone();

        if self.loading_rebase_plan {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(muted_fg)
                        .child("加载 rebase 计划..."),
                )
                .into_any_element();
        }

        let header = h_flex()
            .w_full()
            .flex_none()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .py(px(6.0))
            .border_b_1()
            .border_color(border)
            .bg(bg)
            .child(
                Icon::new(ramag_ui::icons::git_commit())
                    .small()
                    .text_color(theme.accent),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(fg)
                    .child(format!("交互式 Rebase → {onto}")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted_fg)
                    .child(format!("{} 个 commit", self.rebase_todos.len())),
            )
            .child(
                Button::new("vcs-rb-cancel-top")
                    .ghost()
                    .small()
                    .icon(IconName::Close)
                    .label("取消")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.show_rebase_plan = false;
                        this.rebase_todos.clear();
                        cx.notify();
                    })),
            );

        let col_header = h_flex()
            .w_full()
            .flex_none()
            .px(px(10.0))
            .py(px(4.0))
            .border_b_1()
            .border_color(border)
            .gap(px(8.0))
            .child(
                div()
                    .w(px(80.0))
                    .text_xs()
                    .text_color(muted_fg)
                    .child("操作"),
            )
            .child(
                div()
                    .w(px(64.0))
                    .text_xs()
                    .text_color(muted_fg)
                    .child("Hash"),
            )
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .text_color(muted_fg)
                    .child("提交说明"),
            )
            .child(
                div()
                    .w(px(44.0))
                    .text_xs()
                    .text_color(muted_fg)
                    .child("排序"),
            );

        let entity = cx.entity();
        let todos = self.rebase_todos.clone();
        let total = todos.len();
        let rows: Vec<AnyElement> = todos
            .iter()
            .enumerate()
            .map(|(idx, todo)| {
                rebase_todo_row(
                    idx,
                    todo.action,
                    &todo.hash,
                    &todo.subject,
                    total,
                    busy,
                    entity.clone(),
                    cx,
                )
            })
            .collect();

        let body = div()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .child(v_flex().p(px(4.0)).children(rows));

        let footer = h_flex()
            .w_full()
            .flex_none()
            .items_center()
            .justify_end()
            .gap(px(8.0))
            .px(px(12.0))
            .py(px(8.0))
            .border_t_1()
            .border_color(border)
            .bg(bg)
            .child(
                div()
                    .flex_1()
                    .text_xs()
                    .text_color(muted_fg)
                    .child("修改操作或调整顺序，点击「执行 Rebase」应用"),
            )
            .child(
                Button::new("vcs-rb-abort")
                    .ghost()
                    .small()
                    .label("取消")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.show_rebase_plan = false;
                        this.rebase_todos.clear();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("vcs-rb-execute")
                    .primary()
                    .small()
                    .icon(IconName::Play)
                    .label("执行 Rebase")
                    .disabled(busy || self.rebase_todos.is_empty())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.execute_interactive_rebase(cx);
                    })),
            );

        v_flex()
            .size_full()
            .min_h_0()
            .bg(bg)
            .child(header)
            .child(col_header)
            .child(body)
            .child(footer)
            .into_any_element()
    }

    /// 修改某个 todo 的 action（dropdown 回调调用）
    pub(super) fn change_rebase_action(
        &mut self,
        idx: usize,
        action: RebaseAction,
        cx: &mut Context<Self>,
    ) {
        if let Some(todo) = self.rebase_todos.get_mut(idx) {
            todo.action = action;
            cx.notify();
        }
    }

    /// 上移 / 下移 todo（up=true 往前挪）
    pub(super) fn move_rebase_todo(&mut self, idx: usize, up: bool, cx: &mut Context<Self>) {
        let todos = &mut self.rebase_todos;
        if up && idx > 0 {
            todos.swap(idx, idx - 1);
        } else if !up && idx + 1 < todos.len() {
            todos.swap(idx, idx + 1);
        }
        cx.notify();
    }
}

/// 单行 todo 渲染：[action dropdown] [hash] [subject] [↑] [↓]
#[allow(clippy::too_many_arguments)]
fn rebase_todo_row(
    idx: usize,
    action: RebaseAction,
    hash: &str,
    subject: &str,
    total: usize,
    busy: bool,
    entity: gpui::Entity<VcsView>,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let theme = cx.theme();
    let muted_fg = theme.muted_foreground;
    let fg = theme.foreground;
    let hover_bg = theme.muted;
    let mono = theme.mono_font_family.clone();
    let short_hash = &hash[..hash.len().min(7)];
    let subject_owned = subject.to_string();
    let row_id = SharedString::from(format!("vcs-rb-row-{idx}-{}", short_hash));

    let action_label_color = match action {
        RebaseAction::Drop => theme.danger,
        RebaseAction::Squash | RebaseAction::Fixup => theme.accent,
        _ => fg,
    };

    let entity_a = entity.clone();
    let action_btn = Button::new(SharedString::from(format!("vcs-rb-action-{idx}")))
        .ghost()
        .xsmall()
        .label(action.label_zh())
        .w(px(72.0))
        .text_color(action_label_color)
        .dropdown_menu(move |mut menu: PopupMenu, _: &mut Window, _| {
            for a in all_rebase_actions() {
                let ent = entity_a.clone();
                menu = menu.item(PopupMenuItem::new(a.label_zh()).on_click(
                    move |_: &ClickEvent, _: &mut Window, app: &mut App| {
                        ent.update(app, |this, cx| {
                            this.change_rebase_action(idx, a, cx);
                        });
                    },
                ));
            }
            menu
        });

    let entity_up = entity.clone();
    let entity_dn = entity.clone();

    div()
        .id(row_id)
        .w_full()
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(6.0))
        .py(px(2.0))
        .rounded(px(3.0))
        .hover(move |this| this.bg(hover_bg))
        .child(action_btn)
        .child(
            div()
                .w(px(64.0))
                .flex_none()
                .text_xs()
                .font_family(mono)
                .text_color(muted_fg)
                .child(short_hash.to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(fg)
                .overflow_hidden()
                .text_ellipsis()
                .child(subject_owned),
        )
        .child(
            div()
                .flex()
                .gap(px(2.0))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    Button::new(SharedString::from(format!("vcs-rb-up-{idx}")))
                        .ghost()
                        .xsmall()
                        .icon(IconName::ArrowUp)
                        .tooltip("上移（先 rebase）")
                        .disabled(busy || idx == 0)
                        .on_click(move |_: &ClickEvent, _: &mut Window, app: &mut App| {
                            entity_up.update(app, |this, cx| {
                                this.move_rebase_todo(idx, true, cx);
                            });
                        }),
                )
                .child(
                    Button::new(SharedString::from(format!("vcs-rb-dn-{idx}")))
                        .ghost()
                        .xsmall()
                        .icon(IconName::ArrowDown)
                        .tooltip("下移（后 rebase）")
                        .disabled(busy || idx + 1 >= total)
                        .on_click(move |_: &ClickEvent, _: &mut Window, app: &mut App| {
                            entity_dn.update(app, |this, cx| {
                                this.move_rebase_todo(idx, false, cx);
                            });
                        }),
                ),
        )
        .into_any_element()
}

fn all_rebase_actions() -> [RebaseAction; 6] {
    [
        RebaseAction::Pick,
        RebaseAction::Reword,
        RebaseAction::Edit,
        RebaseAction::Squash,
        RebaseAction::Fixup,
        RebaseAction::Drop,
    ]
}
