//! History：commit 列表分页 + 搜索（关键词 / `@作者` / `7d`/`1m`）+ 单文件历史 banner。
//! viewing_commit.is_some() 时整区切到 commit_detail

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement, Styled,
    div, px, uniform_list,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    scroll::ScrollableElement as _,
    v_flex,
};

use ramag_domain::entities::Commit;

use super::commit_graph::CommitGraphRow;
use super::helpers::{build_commit_lanes, render_commit_row};
use super::vcs_view::VcsView;

impl VcsView {
    /// 历史视图：commit list / 详情视图（点击 [👁] 后）/ reflog（[📜] toggle 后）
    pub(super) fn render_history_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted_fg = theme.muted_foreground;
        let accent = theme.accent;
        let border = theme.border;
        let mono = theme.mono_font_family.clone();
        let busy = self.busy;

        let path_banner: AnyElement = if let Some(path) = &self.history_path_filter {
            let mut chip_bg = accent;
            chip_bg.a = 0.14;
            h_flex()
                .gap(px(8.0))
                .items_center()
                .px(px(10.0))
                .py(px(4.0))
                .rounded(px(6.0))
                .bg(chip_bg)
                .mb(px(8.0))
                .child(
                    Icon::new(ramag_ui::icons::scroll_text())
                        .small()
                        .text_color(accent),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_xs()
                        .text_color(fg)
                        .font_family(mono.clone())
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(format!("正在看 {path} 的历史")),
                )
                .child(
                    Button::new("vcs-history-clear-path")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .tooltip("清除单文件过滤，回到全仓库历史")
                        .disabled(busy)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.clear_history_path_filter(cx);
                        })),
                )
                .into_any_element()
        } else {
            div().into_any_element()
        };

        // 统一走三栏布局：reflog 与 commit 模式均保留左栏分支视图
        // 中栏内容由 showing_reflog 切换；右栏 commit 详情仅 commit 模式可见
        let body: AnyElement =
            self.render_history_three_panel(border, fg, muted_fg, accent, mono, busy, cx);

        v_flex()
            .size_full()
            .px(px(12.0))
            .pt(px(6.0))
            .pb(px(8.0))
            .gap(px(0.0))
            .child(path_banner)
            .child(body)
            .into_any_element()
    }

    /// commit / reflog 列表共用搜索行
    fn render_history_search_row(
        &self,
        busy: bool,
        muted_fg: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let reflog_btn = Button::new("vcs-history-reflog-toggle")
            .ghost()
            .small()
            .icon(ramag_ui::icons::scroll_text())
            .tooltip(if self.showing_reflog {
                "切回 commit 历史"
            } else {
                "查看 reflog（找回丢失 commit）"
            })
            .disabled(busy)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.toggle_reflog(cx);
            }));
        h_flex()
            .gap(px(6.0))
            .items_center()
            .h(px(36.0))
            .flex_none()
            .px(px(8.0))
            .child(reflog_btn)
            .child(Icon::new(IconName::Search).small().text_color(muted_fg))
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.history_search_input)
                        .small()
                        .into_any_element(),
                ),
            )
            .child(
                Button::new("vcs-history-search")
                    .ghost()
                    .small()
                    .icon(IconName::ArrowRight)
                    .tooltip("应用搜索条件")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.apply_history_search(cx);
                    })),
            )
            .into_any_element()
    }

    /// 双栏：左分支 / 右半（含 commit graph + 内部 detail resizable）。
    /// 外层永远 2 children 与上半共用 `ide_left_resize` 同步对齐；reflog 模式右栏 detail 隐藏
    #[allow(clippy::too_many_arguments)]
    fn render_history_three_panel(
        &self,
        border: gpui::Hsla,
        fg: gpui::Hsla,
        muted_fg: gpui::Hsla,
        accent: gpui::Hsla,
        mono: gpui::SharedString,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let left = self.render_history_left_pane(border, cx);
        let middle = if self.showing_reflog {
            self.render_reflog_middle_pane(busy, muted_fg, cx)
        } else {
            self.render_history_middle_pane(fg, muted_fg, accent, border, mono, busy, cx)
        };
        // reflog 行没有完整 commit 元数据；detail 面板对其无意义，强制隐藏
        let show_detail = !self.showing_reflog && self.viewing_commit.is_some();

        // 右半内容：默认仅 commit graph；进入详情时变成内部 h_resizable（middle | detail）
        let right_part: AnyElement = if show_detail {
            let detail = self.render_commit_detail_view(cx);
            gpui_component::resizable::h_resizable("vcs-history-detail-split")
                .with_state(&self.detail_resize)
                .child(
                    gpui_component::resizable::resizable_panel()
                        .child(div().size_full().min_w_0().child(middle)),
                )
                .child(
                    gpui_component::resizable::resizable_panel()
                        .size(px(280.0))
                        .size_range(px(220.0)..px(720.0))
                        .child(div().size_full().child(detail)),
                )
                .into_any_element()
        } else {
            div().size_full().min_w_0().child(middle).into_any_element()
        };

        // 外层与上半共用 `ide_left_resize`：两边都是 2 子项（左 / 右半），共享 state
        // → 上下左栏宽度 100% 同步对齐（拖一边另一边跟随，IDEA / VSCode 标准做法）
        gpui_component::resizable::h_resizable("vcs-history-bottom")
            .with_state(&self.ide_left_resize)
            .child(
                gpui_component::resizable::resizable_panel()
                    .size(px(280.0))
                    .size_range(px(220.0)..px(600.0))
                    .child(
                        div()
                            .size_full()
                            .border_r_1()
                            .border_color(border)
                            .child(left),
                    ),
            )
            .child(
                gpui_component::resizable::resizable_panel()
                    .child(div().size_full().child(right_part)),
            )
            .into_any_element()
    }

    /// 中栏（reflog 模式）：搜索行 + 现有 reflog 列表
    /// 与 commit 中栏共用同一空间，左栏 / 整体三栏框架不变
    fn render_reflog_middle_pane(
        &self,
        busy: bool,
        muted_fg: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .size_full()
            .min_h_0()
            .px(px(8.0))
            .child(self.render_history_search_row(busy, muted_fg, cx))
            .child(div().flex_1().min_h_0().child(self.render_reflog_view(cx)))
            .into_any_element()
    }

    /// 左栏：本地 / 远程分支 + tags 段（复用 sidebar 段渲染，可折叠）
    fn render_history_left_pane(&self, _border: gpui::Hsla, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .id("vcs-history-left-pane")
            .size_full()
            .px(px(8.0))
            .py(px(6.0))
            .gap(px(8.0))
            .overflow_y_scrollbar()
            .child(self.render_local_branches_section(cx))
            .child(self.render_remote_branches_section(cx))
            .child(self.render_tags_section(cx))
            .into_any_element()
    }

    /// 中栏：计数 + 列头 + uniform_list 虚拟化 + 加载更多。列头 / count / footer 在外层非虚拟
    #[allow(clippy::too_many_arguments)]
    fn render_history_middle_pane(
        &self,
        fg: gpui::Hsla,
        muted_fg: gpui::Hsla,
        accent: gpui::Hsla,
        _border: gpui::Hsla,
        mono: gpui::SharedString,
        _busy: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let search_row = self.render_history_search_row(_busy, muted_fg, cx);
        if self.history_commits.is_empty() && self.loading_history {
            return v_flex()
                .size_full()
                .px(px(8.0))
                .child(search_row)
                .child(center_msg("加载中...", muted_fg))
                .into_any_element();
        }
        if self.history_commits.is_empty() {
            return v_flex()
                .size_full()
                .px(px(8.0))
                .child(search_row)
                .child(center_msg("（暂无提交记录）", muted_fg))
                .into_any_element();
        }

        let count = self.history_commits.len();
        let has_more = self.history_has_more;
        let is_loading = self.loading_history;
        // 有更多时加一行哨兵行：滚到底自动触发下一页加载
        let total_rows = count + usize::from(has_more);
        // Rc 共享：commits + graph_rows 喂给 uniform_list 闭包（不每帧 clone 整个 Vec）
        let commits_rc: Rc<Vec<Commit>> = Rc::new(self.history_commits.clone());
        let graph_rc: Rc<Vec<CommitGraphRow>> = Rc::new(build_commit_lanes(&self.history_commits));

        let body = uniform_list(
            "vcs-history-commits",
            total_rows,
            cx.processor({
                let commits_rc = commits_rc.clone();
                let graph_rc = graph_rc.clone();
                let mono = mono.clone();
                move |this, range: Range<usize>, window, cx| {
                    let selected_id = this
                        .viewing_commit
                        .as_ref()
                        .map(|c| c.id.0.clone())
                        .unwrap_or_default();
                    range
                        .map(|i| {
                            if i == count && has_more {
                                // 哨兵行：滚到底时自动加载下一页
                                if !is_loading {
                                    cx.defer_in(window, move |this, _, cx| {
                                        this.load_history_page(count, cx);
                                    });
                                }
                                return div()
                                    .h(px(28.0))
                                    .flex_none()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .child(if is_loading {
                                        "加载中..."
                                    } else {
                                        "加载更多..."
                                    })
                                    .into_any_element();
                            }
                            let is_selected = commits_rc[i].id.0 == selected_id;
                            div()
                                .h(px(28.0))
                                .flex_none()
                                .child(render_commit_row(
                                    &commits_rc[i],
                                    &graph_rc[i],
                                    mono.clone(),
                                    fg,
                                    muted_fg,
                                    accent,
                                    is_selected,
                                    cx,
                                ))
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                }
            }),
        )
        .track_scroll(&self.history_scroll)
        .flex_1();

        let footer: AnyElement = if !has_more {
            div()
                .flex_none()
                .py(px(8.0))
                .flex()
                .justify_center()
                .text_xs()
                .text_color(muted_fg)
                .child("— 已到底 —")
                .into_any_element()
        } else {
            div().flex_none().into_any_element()
        };

        v_flex()
            .size_full()
            .min_h_0()
            .px(px(8.0))
            .child(search_row)
            .child(body)
            .child(footer)
            .into_any_element()
    }
}

fn center_msg(msg: &'static str, muted_fg: gpui::Hsla) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .text_color(muted_fg)
        .child(msg)
        .into_any_element()
}
