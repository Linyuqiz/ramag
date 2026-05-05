//! History 列表的单行渲染（IDEA Git 风格）
//!
//! 拆出来让 helpers/mod.rs 不超 600 行红线。

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, px,
};
use gpui_component::{
    ActiveTheme, h_flex,
    menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem},
};
use ramag_domain::entities::{Commit, ResetKind};

use super::super::commit_graph::{CommitGraphRow, lane_color, render_lane_gutter};
use super::super::vcs_view::VcsView;

/// History 列表的单行渲染（IDEA Git 风格列化）：
/// `[lane gutter] | subject + [refs] | author | date | hash`
///
/// 左键点击打开 commit 详情；右键唤出操作菜单（cherry-pick / revert / reset）。
/// 行高紧凑（28px）、字体 13/12，便于一屏看更多 commit。
#[allow(clippy::too_many_arguments)]
pub(in crate::views) fn render_commit_row(
    c: &Commit,
    graph: &CommitGraphRow,
    mono: SharedString,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    accent: gpui::Hsla,
    selected: bool,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let time_str = relative_time(&c.author.timestamp);
    let author_short: String = c.author.name.chars().take(20).collect::<String>();
    let dot_color = lane_color(graph.lane);
    let hover_bg = cx.theme().muted;
    let mut sel_bg = accent;
    sel_bg.a = 0.12;

    let entity = cx.entity().clone();
    let cid = c.id.0.clone();

    // refs chips（紧贴 subject 后）
    let mut refs_row = h_flex().gap(px(4.0)).flex_none();
    for r in &c.refs {
        refs_row = refs_row.child(ref_chip(r, accent));
    }

    let row_id = SharedString::from(format!("vcs-commit-row-{}", &cid[..cid.len().min(12)]));

    // 左键：打开 commit 详情（右侧面板）
    let cid_click = cid.clone();
    let on_click_handler = cx.listener(move |this, _: &ClickEvent, _, cx| {
        this.load_commit_detail(cid_click.clone(), cx);
    });

    let mut row = h_flex()
        .id(row_id)
        .w_full()
        .py(px(2.0))
        .items_center()
        .gap(px(0.0))
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .on_click(on_click_handler)
        // 列 1：lane gutter
        .child(render_lane_gutter(graph))
        // 列 2：subject + refs（flex_1 撑开）
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(px(6.0))
                .px(px(8.0))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(fg)
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(c.subject.clone()),
                )
                .child(refs_row),
        )
        // 列 3：author
        .child(
            div()
                .flex_none()
                .w(px(140.0))
                .px(px(6.0))
                .text_xs()
                .text_color(muted_fg)
                .overflow_hidden()
                .text_ellipsis()
                .child(author_short),
        )
        // 列 4：date
        .child(
            div()
                .flex_none()
                .w(px(96.0))
                .px(px(6.0))
                .text_xs()
                .text_color(muted_fg)
                .child(time_str),
        )
        // 列 5：hash
        .child(
            div()
                .flex_none()
                .w(px(70.0))
                .px(px(6.0))
                .text_xs()
                .font_family(mono.clone())
                .text_color({
                    let mut col = dot_color;
                    col.a = 0.85;
                    col
                })
                .child(c.id.short().to_string()),
        );

    if selected {
        row = row.bg(sel_bg);
    }

    // 右键菜单：cherry-pick / revert / reset
    row.context_menu({
        let entity = entity.clone();
        let cid = cid.clone();
        move |menu: PopupMenu, _, _| {
            let (e1, c1) = (entity.clone(), cid.clone());
            let (e2, c2) = (entity.clone(), cid.clone());
            let (e3, c3) = (entity.clone(), cid.clone());
            menu.item(PopupMenuItem::new("Cherry-pick 到当前 HEAD").on_click(
                move |_, window, app| {
                    use crate::views::confirm_dialogs::open_confirm_dialog;
                    let short: String = c1.chars().take(7).collect();
                    let c = c1.clone();
                    open_confirm_dialog(
                        e1.clone(),
                        "Cherry-pick 这个 commit？",
                        format!(
                            "将把「{short}」拣选到当前 HEAD。\n\
                                 有冲突时会进入 cherry-pick 进行中状态。"
                        ),
                        "Cherry-pick",
                        false,
                        move |this, cx| this.run_cherry_pick(c, cx),
                        window,
                        app,
                    );
                },
            ))
            .item(PopupMenuItem::new("Revert（生成反向 commit）").on_click(
                move |_, window, app| {
                    use crate::views::confirm_dialogs::open_confirm_dialog;
                    let short: String = c2.chars().take(7).collect();
                    let c = c2.clone();
                    open_confirm_dialog(
                        e2.clone(),
                        "Revert 这个 commit？",
                        format!(
                            "将生成一个反向 commit 撤销「{short}」的改动（不改写历史，安全）。\n\
                                 有冲突时会进入 revert 进行中状态。"
                        ),
                        "Revert",
                        false,
                        move |this, cx| this.run_revert(c, cx),
                        window,
                        app,
                    );
                },
            ))
            .item(
                PopupMenuItem::new("Reset --mixed 到此").on_click(move |_, window, app| {
                    use crate::views::confirm_dialogs::open_confirm_dialog;
                    let short: String = c3.chars().take(7).collect();
                    let c = c3.clone();
                    open_confirm_dialog(
                        e3.clone(),
                        "Reset --mixed？",
                        format!(
                            "将 HEAD 移到「{short}」并重置暂存区（工作区保留）。\n\
                             未提交的暂存内容会回到未暂存状态。"
                        ),
                        "Reset",
                        false,
                        move |this, cx| this.run_reset(c, ResetKind::Mixed, cx),
                        window,
                        app,
                    );
                }),
            )
        }
    })
    .into_any_element()
}

/// 把 chrono::DateTime 渲染成「3 天前 / 2 小时前 / 刚刚」相对时间
fn relative_time(ts: &chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let secs = (now - *ts).num_seconds();
    if secs < 60 {
        return "刚刚".into();
    }
    if secs < 3600 {
        return format!("{} 分钟前", secs / 60);
    }
    if secs < 86400 {
        return format!("{} 小时前", secs / 3600);
    }
    if secs < 86400 * 30 {
        return format!("{} 天前", secs / 86400);
    }
    if secs < 86400 * 365 {
        return format!("{} 个月前", secs / (86400 * 30));
    }
    ts.format("%Y-%m-%d").to_string()
}

/// commit refs 标签：根据 ref 名前缀决定颜色（HEAD / origin/* / tag: *）
fn ref_chip(name: &str, accent: gpui::Hsla) -> AnyElement {
    // tag 名习惯以 "tag: " 前缀（git log --decorate）
    let (label, tone) = if let Some(rest) = name.strip_prefix("tag: ") {
        (rest.to_string(), gpui::hsla(40.0 / 360.0, 0.7, 0.55, 1.0))
    } else if name.starts_with("HEAD") {
        (name.to_string(), gpui::hsla(140.0 / 360.0, 0.55, 0.45, 1.0))
    } else if name.contains('/') {
        // remote-tracking：origin/main 等
        (name.to_string(), gpui::hsla(220.0 / 360.0, 0.6, 0.55, 1.0))
    } else {
        (name.to_string(), accent)
    };
    let mut bg = tone;
    bg.a = 0.16;
    div()
        .px(px(6.0))
        .py(px(1.0))
        .rounded(px(4.0))
        .bg(bg)
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(tone)
        .child(label)
        .into_any_element()
}
