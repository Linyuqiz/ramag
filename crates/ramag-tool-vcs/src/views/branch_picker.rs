//! 分支 dropdown：把扁平分支列表按 `/` 路径分组成 submenu 嵌套
//!
//! 拆分自 `ide_layout.rs`，保持每文件 ≤ 600 行。
//! 单段名（无 `/`）直接 `m.item`；多项同前缀进 submenu，单项不分组（避免一项也嵌套）

use std::collections::BTreeMap;

use gpui::{ClickEvent, Entity, ParentElement, SharedString, Styled, Window, px};
use gpui_component::{
    Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    v_flex,
};

use super::helpers::BranchOp;
use super::vcs_view::VcsView;

/// 分支 leaf：(完整名 / 是否 HEAD / 上游同步信息文本如 "↑3 ↓1"，None=无)
pub(super) type BranchLeaf = (String, bool, Option<String>);

/// 分支显示名截断：超长名（如 `sdafasd-sadfsdaf-asdfsadfa-...`）会撑破 PopupMenu 宽度，
/// 超过阈值就用中间省略 `头20…尾15`，保留首尾辨识度，鼠标悬停 tooltip 仍可补全（暂未实现）
fn truncate_branch_display(s: &str) -> String {
    const MAX_CHARS: usize = 40;
    const HEAD_KEEP: usize = 22;
    const TAIL_KEEP: usize = 15;
    let count = s.chars().count();
    if count <= MAX_CHARS {
        return s.to_string();
    }
    let head: String = s.chars().take(HEAD_KEEP).collect();
    let tail: String = s.chars().skip(count - TAIL_KEEP).collect();
    format!("{head}…{tail}")
}

/// 分支按 / 路径分组渲染：单段名直接列出；多项同前缀走原生 submenu（hover 展开侧菜单，
/// 父菜单不关闭，子菜单自身可滚动）。
///
/// 为什么用 submenu：PopupMenu 的 `Item.on_click` 必走 confirm → dismiss，
/// 任何 inline 折叠展开方案都会要么关菜单（item dismiss）、要么菜单不刷新（menu_items 静态），
/// 只有 `submenu`（gpui-component 内置）能做到"点击/悬停打开 + 父菜单保持开启 + 子项点击 checkout 并关闭整个菜单链"。
///
/// 注意：父级 PopupMenu **不能**调 `.scrollable(true)`，否则 submenu 不工作（gpui-component 限制）。
/// 调用方需确保父菜单内容能在窗口高度内显示——分支用 group 收纳后通常没问题。
///
/// `is_remote=true` 时叶子前缀 ↗ 标记，且 click 时不限制 head_flag（远程分支总是允许 checkout）。
pub(super) fn render_branches_grouped(
    mut m: PopupMenu,
    items: &[BranchLeaf],
    is_remote: bool,
    entity: Entity<VcsView>,
    window: &mut Window,
    cx: &mut gpui::Context<PopupMenu>,
) -> PopupMenu {
    let mut singles: Vec<BranchLeaf> = Vec::new();
    let mut groups: BTreeMap<String, Vec<BranchLeaf>> = BTreeMap::new();
    for (name, is_head, sync) in items {
        if let Some(slash) = name.find('/') {
            let prefix = name[..slash].to_string();
            let rest = name[slash + 1..].to_string();
            groups
                .entry(prefix)
                .or_default()
                .push((rest, *is_head, sync.clone()));
        } else {
            singles.push((name.clone(), *is_head, sync.clone()));
        }
    }
    // 单段名（无 /）直接列出
    for (name, is_head, sync) in &singles {
        m = push_branch_leaf(m, name, name, *is_head, is_remote, sync, entity.clone());
    }
    // 有 / 路径前缀：每个 prefix 一个 submenu。submenu 自带 ▸ 图标 + hover 打开
    for (prefix, group_items) in groups {
        let entity_for_sub = entity.clone();
        let prefix_for_sub = prefix.clone();
        let group_items_owned = group_items;
        m = m.submenu(
            SharedString::from(prefix),
            window,
            cx,
            move |mut sub, _w, _c| {
                // 子菜单单独可滚动（远程 origin/* 可能很多）
                sub = sub.scrollable(true).max_h(px(360.0));
                for (rest, is_head, sync) in group_items_owned.iter() {
                    let full = format!("{prefix_for_sub}/{rest}");
                    sub = push_branch_leaf(
                        sub,
                        &full,
                        rest,
                        *is_head,
                        is_remote,
                        sync,
                        entity_for_sub.clone(),
                    );
                }
                sub
            },
        );
    }
    m
}

/// 打开「新建分支」对话框：input + 源分支 dropdown（默认当前 HEAD，可手动选）
///
/// 调用前会 reset `view.create_branch_base = None`（让 dropdown 显示当前 HEAD 名）；
/// 用户选 dropdown 项后写入 base；点「创建」时 handle_create_branch 用此 base
pub(super) fn open_new_branch_dialog(
    view: Entity<VcsView>,
    head_name: String,
    local: Vec<(String, bool)>,
    remote: Vec<String>,
    window: &mut Window,
    app: &mut gpui::App,
) {
    // reset：每次打开对话框都从 HEAD 开始（不 stick 上次的选择）
    view.update(app, |this, cx| {
        this.create_branch_base = None;
        cx.notify();
    });
    let title = SharedString::from("新建分支");
    window.open_dialog(app, move |dialog, _window, _app| {
        let view = view.clone();
        let head_name = head_name.clone();
        let local = local.clone();
        let remote = remote.clone();
        dialog
            .title(title.clone())
            .margin_top(px(180.0))
            .content({
                let view = view.clone();
                let head_name = head_name.clone();
                move |c, _, app| {
                    let cur_base = view.read(app).create_branch_base.clone();
                    let base_label = cur_base.unwrap_or_else(|| head_name.clone());
                    let inp = view.read(app).create_branch_input.clone();
                    let local_for_dd = local.clone();
                    let remote_for_dd = remote.clone();
                    let view_for_dd = view.clone();
                    let head_for_reset = head_name.clone();
                    let _ = app;
                    let base_btn = Button::new("vcs-new-br-base")
                        .outline()
                        .small()
                        .label(format!("基于：{base_label} ▾"))
                        .dropdown_menu_with_anchor(
                            gpui::Anchor::TopLeft,
                            move |mut m, window, cx| {
                                // 父级不可 scrollable —— 否则 submenu 不工作（gpui-component 限制）
                                // 限宽避免超长分支名撑破菜单（叶子内部已做中间省略截断）
                                m = m.max_w(px(420.0));
                                // 重置项：选当前 HEAD
                                let v_reset = view_for_dd.clone();
                                let h_reset = head_for_reset.clone();
                                m = m.item(
                                    PopupMenuItem::new(format!("✓  {h_reset}（当前 HEAD）"))
                                        .on_click(move |_, _, app| {
                                            v_reset.update(app, |this, cx| {
                                                this.set_create_branch_base(None, cx);
                                            });
                                        }),
                                );
                                m = m.separator();
                                m = m.item(PopupMenuItem::label("本地"));
                                m = render_base_branches_grouped(
                                    m,
                                    &local_for_dd
                                        .iter()
                                        .map(|(n, _)| n.clone())
                                        .collect::<Vec<_>>(),
                                    false,
                                    view_for_dd.clone(),
                                    window,
                                    cx,
                                );
                                if !remote_for_dd.is_empty() {
                                    m = m.separator();
                                    m = m.item(PopupMenuItem::label("远程"));
                                    m = render_base_branches_grouped(
                                        m,
                                        &remote_for_dd,
                                        true,
                                        view_for_dd.clone(),
                                        window,
                                        cx,
                                    );
                                }
                                m
                            },
                        );
                    c.child(
                        v_flex()
                            .gap(px(8.0))
                            .py(px(6.0))
                            .child(Input::new(&inp).small())
                            .child(base_btn),
                    )
                }
            })
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        Button::new("vcs-new-br-cancel")
                            .ghost()
                            .small()
                            .label("取消")
                            .on_click(|_: &ClickEvent, w, app| w.close_dialog(app)),
                    )
                    .child(
                        Button::new("vcs-new-br-ok")
                            .primary()
                            .small()
                            .label("创建")
                            .on_click({
                                let v = view.clone();
                                move |_: &ClickEvent, w, app| {
                                    v.update(app, |this, cx| {
                                        this.handle_create_branch(cx);
                                    });
                                    w.close_dialog(app);
                                }
                            }),
                    ),
            )
    });
}

/// 新建分支对话框「基于」下拉的分组渲染：与主选择器同款 submenu 模式（hover 展开、父菜单不关），
/// 但叶子点击调 `set_create_branch_base(Some(name))` 而非 checkout；不需要 HEAD ✓ 标记
/// （HEAD 已在调用方作为顶部独立"✓ {head}（当前 HEAD）"重置项渲染）
fn render_base_branches_grouped(
    mut m: PopupMenu,
    names: &[String],
    is_remote: bool,
    view: Entity<VcsView>,
    window: &mut Window,
    cx: &mut gpui::Context<PopupMenu>,
) -> PopupMenu {
    let mut singles: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in names {
        if let Some(slash) = name.find('/') {
            let prefix = name[..slash].to_string();
            let rest = name[slash + 1..].to_string();
            groups.entry(prefix).or_default().push(rest);
        } else {
            singles.push(name.clone());
        }
    }
    for name in &singles {
        m = push_base_leaf(m, name, name, is_remote, view.clone());
    }
    for (prefix, rests) in groups {
        let view_for_sub = view.clone();
        let prefix_for_sub = prefix.clone();
        m = m.submenu(
            SharedString::from(prefix),
            window,
            cx,
            move |mut sub, _w, _c| {
                sub = sub.scrollable(true).max_h(px(360.0));
                for rest in rests.iter() {
                    let full = format!("{prefix_for_sub}/{rest}");
                    sub = push_base_leaf(sub, &full, rest, is_remote, view_for_sub.clone());
                }
                sub
            },
        );
    }
    m
}

/// 给「基于」下拉添加一个分支叶子：点击调 set_create_branch_base 写入选中的分支名
fn push_base_leaf(
    m: PopupMenu,
    full_name: &str,
    display: &str,
    is_remote: bool,
    view: Entity<VcsView>,
) -> PopupMenu {
    let prefix = if is_remote { "↗  " } else { "    " };
    let label = format!("{prefix}{}", truncate_branch_display(display));
    let n = full_name.to_string();
    m.item(PopupMenuItem::new(label).on_click(move |_, _, app| {
        let n = n.clone();
        view.update(app, |this, cx| {
            this.set_create_branch_base(Some(n), cx);
        });
    }))
}

/// 在 PopupMenu 上加一个分支 item：HEAD 加 ✓ / 远程加 ↗ / 末尾追加 ahead/behind 同步信息
fn push_branch_leaf(
    m: PopupMenu,
    full_name: &str,
    display: &str,
    is_head: bool,
    is_remote: bool,
    sync: &Option<String>,
    entity: Entity<VcsView>,
) -> PopupMenu {
    let prefix = if is_remote {
        "↗  "
    } else if is_head {
        "✓  "
    } else {
        "    "
    };
    let suffix = match sync {
        Some(s) if !s.is_empty() => format!("    {s}"),
        _ => String::new(),
    };
    let label = format!("{prefix}{}{suffix}", truncate_branch_display(display));
    let n = full_name.to_string();
    m.item(PopupMenuItem::new(label).on_click(move |_, w, app| {
        if is_head && !is_remote {
            return;
        }
        let n = n.clone();
        entity.update(app, |this, cx| {
            this.confirm_branch_op(BranchOp::Checkout(n), w, cx);
        });
    }))
}
