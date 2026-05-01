//! Diff / Blame 视图渲染（workspace 右侧主区）
//!
//! 抽自 workspace_panel.rs（让其不超 600 行）。本模块只负责右半区——
//! 顶部按钮组（视图切换 / Blame toggle / Stage|Unstage 选中）+ 中间内容（diff 或 blame）。

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};

use super::helpers::{FileTabSource, GroupKind};
use super::vcs_view::VcsView;

impl VcsView {
    /// Diff 面板：当前选中文件的 unified/split diff 或 blame（无选中时显空状态提示）
    pub(super) fn render_diff_block(&self, cx: &mut Context<Self>) -> AnyElement {
        // 提前 clone 主题字段，避免后续 cx.listener 借用冲突
        let (fg, muted_fg, accent, muted_bg, border, mono) = {
            let theme = cx.theme();
            (
                theme.foreground,
                theme.muted_foreground,
                theme.accent,
                theme.muted,
                theme.border,
                theme.mono_font_family.clone(),
            )
        };

        // 优先级：active file_tab → 派生 path / kind_tag / kind（用于 stage 选中按钮）
        // file_tab 是 Commit / ProjectFiles 时 selected_file 为 None，此时从 tab 推 kind_tag
        let active_tab = self.active_file_tab_idx.and_then(|i| self.file_tabs.get(i));
        let Some(tab) = active_tab else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .px(px(40.0))
                .text_sm()
                .text_color(muted_fg)
                .child("选中左侧文件查看变更")
                .into_any_element();
        };
        // Commit tab 走只读 diff 路径（不开 stage/unstage）；Changes 走 GroupKind 原逻辑
        let (path, kind, kind_tag): (String, GroupKind, String) = match &tab.source {
            FileTabSource::Changes(k) => {
                let tag = match k {
                    GroupKind::Staged => "已暂存",
                    GroupKind::Unstaged => "未暂存",
                    GroupKind::Untracked => "未跟踪",
                    GroupKind::Conflict => "冲突",
                };
                (tab.path.clone(), *k, tag.to_string())
            }
            FileTabSource::Commit { commit_id, .. } => {
                let short: String = commit_id.chars().take(7).collect();
                (
                    tab.path.clone(),
                    GroupKind::Staged, // 占位：Commit diff 走只读路径，kind 仅用作 enum 必填字段
                    format!("Commit {short}"),
                )
            }
            FileTabSource::ProjectFiles => {
                // 不会进这条路（render_main_area 已分流到 render_pf_content）
                return div().into_any_element();
            }
        };
        let kind_copy = kind;
        let header = self.render_diff_header(
            &kind_tag,
            &path,
            kind_copy,
            fg,
            accent,
            mono.clone(),
            border,
            cx,
        );
        let body =
            self.render_diff_body(kind_copy, mono.clone(), fg, muted_fg, muted_bg, accent, cx);
        // blame 不再替换主区：开启 blame 后点行号 → 顶部 banner 展示该行作者
        // 行号 cell 的点击交互仍然有效，无论 showing_blame 与否
        let _ = border;
        let _ = muted_bg;
        let body_layout: AnyElement = div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .w_full()
            .child(body)
            .into_any_element();

        let mut col = v_flex().size_full().min_w_0().child(header);
        if let Some(blame_text) = &self.inline_blame_text {
            col = col.child(render_inline_blame_banner(
                blame_text.clone(),
                accent,
                fg,
                mono,
                cx,
            ));
        }
        col.child(body_layout).into_any_element()
    }

    /// 文件 tab 条（Changes diff 与 ProjectFiles 内容统一显示在主区顶部）
    pub(super) fn render_file_tab_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.file_tabs.is_empty() {
            return div().into_any_element();
        }
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted_fg = theme.muted_foreground;
        let border = theme.border;
        let accent = theme.accent;
        let muted_bg = theme.muted;
        let mut accent_bg = accent;
        accent_bg.a = 0.12;

        let mut bar = h_flex()
            .id("vcs-ftab-bar")
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(border)
            .overflow_x_scroll();

        for (idx, tab) in self.file_tabs.iter().enumerate() {
            let is_active = self.active_file_tab_idx == Some(idx);
            let filename = SharedString::from(
                tab.path
                    .split('/')
                    .next_back()
                    .unwrap_or(&tab.path)
                    .to_string(),
            );
            let tab_id = SharedString::from(format!("vcs-ftab-{idx}"));
            let close_id = SharedString::from(format!("vcs-ftab-close-{idx}"));
            let dot_color = match &tab.source {
                FileTabSource::Changes(GroupKind::Staged) => accent,
                FileTabSource::Changes(GroupKind::Unstaged) => {
                    gpui::hsla(40.0 / 360.0, 0.7, 0.55, 1.0)
                }
                FileTabSource::Changes(GroupKind::Untracked) => muted_fg,
                FileTabSource::Changes(GroupKind::Conflict) => gpui::hsla(0.0, 0.65, 0.55, 1.0),
                FileTabSource::ProjectFiles => gpui::hsla(210.0 / 360.0, 0.6, 0.55, 1.0),
                FileTabSource::Commit { .. } => gpui::hsla(280.0 / 360.0, 0.55, 0.55, 1.0),
            };
            let path_for_click = tab.path.clone();
            let source_for_click = tab.source.clone();

            let mut tab_el = h_flex()
                .id(tab_id)
                .items_center()
                .gap(px(4.0))
                .px(px(10.0))
                .py(px(4.0))
                .border_r_1()
                .border_color(border)
                .cursor_pointer()
                .child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(dot_color))
                .child(
                    div()
                        .text_xs()
                        .text_color(if is_active { fg } else { muted_fg })
                        .child(filename),
                )
                .child(
                    Button::new(close_id)
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.close_file_tab(idx, cx);
                        })),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    match source_for_click.clone() {
                        FileTabSource::Changes(kind) => {
                            this.select_file(path_for_click.clone(), kind, cx);
                        }
                        FileTabSource::ProjectFiles => {
                            this.select_pf_file(path_for_click.clone(), cx);
                        }
                        FileTabSource::Commit { commit_id, .. } => {
                            this.select_commit_file(path_for_click.clone(), commit_id, cx);
                        }
                    }
                }));

            tab_el = if is_active {
                tab_el.bg(accent_bg)
            } else {
                tab_el.hover(move |s| s.bg(muted_bg))
            };
            bar = bar.child(tab_el);
        }
        bar.into_any_element()
    }

    /// 顶部 header：kind 徽标 + 路径 + Stage/Unstage 选中 + Blame toggle + 视图切换
    #[allow(clippy::too_many_arguments)]
    fn render_diff_header(
        &self,
        kind_tag: &str,
        path: &str,
        kind: GroupKind,
        fg: gpui::Hsla,
        accent: gpui::Hsla,
        mono: SharedString,
        border: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _ = kind;
        // Blame toggle：Changes 与 Commit tab 都支持（toggle_blame 内部按 selected_file/commit_file 取 path）
        let blame_supported =
            matches!(kind, GroupKind::Staged | GroupKind::Unstaged) || self.viewing_commit.is_some();
        let blame_btn = Button::new("vcs-diff-blame-toggle")
            .ghost()
            .xsmall()
            .icon(IconName::Eye)
            .tooltip(if self.showing_blame {
                "关闭 blame（不再点行号查看作者）"
            } else {
                "启用 blame（点行号查看该行最后改人）"
            })
            .disabled(!blame_supported)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.toggle_blame(cx);
            }));
        // Diff 视图模式：图标 toggle（点击在「标准」与「全文件」间切换）
        let is_full = matches!(self.diff_view_mode, super::helpers::DiffViewMode::FullFile);
        let view_mode_btn = Button::new("vcs-diff-view-mode")
            .ghost()
            .xsmall()
            .icon(if is_full {
                ramag_ui::icons::list_filter()
            } else {
                ramag_ui::icons::scroll_text()
            })
            .tooltip(if is_full {
                "回到「标准」（带少量上下文）"
            } else {
                "展示「全文件」（完整文件 + 高亮变更）"
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.set_diff_view_mode(this.diff_view_mode.toggled(), cx);
            }));

        h_flex()
            .gap(px(6.0))
            .items_center()
            .px(px(10.0))
            .py(px(5.0))
            .border_b_1()
            .border_color(border)
            .child(
                div()
                    .px(px(6.0))
                    .py(px(1.0))
                    .rounded(px(3.0))
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(accent)
                    .bg({
                        let mut c = accent;
                        c.a = 0.14;
                        c
                    })
                    .child(kind_tag.to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(fg)
                    .font_family(mono)
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(path.to_string()),
            )
            .child(blame_btn)
            .child(view_mode_btn)
            .into_any_element()
    }

    /// 中间 body：diff（loading / 占位 / unified or split）；blame 单独由 render_diff_block 摆放
    #[allow(clippy::too_many_arguments)]
    fn render_diff_body(
        &self,
        kind: GroupKind,
        mono: SharedString,
        fg: gpui::Hsla,
        muted_fg: gpui::Hsla,
        muted_bg: gpui::Hsla,
        _accent: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.loading_diff {
            return placeholder("拉取中...", muted_fg);
        }
        if matches!(kind, GroupKind::Untracked) {
            return placeholder("（未跟踪文件，先 Stage 后查看 diff）", muted_fg);
        }
        if matches!(kind, GroupKind::Conflict) {
            return placeholder("（冲突文件需要三栏解决器，v0.2 加入）", muted_fg);
        }
        let Some(d) = &self.current_diff else {
            return placeholder("（无差异）", muted_fg);
        };
        // Changes（Staged/Unstaged）开行级选择 + 中间 ↶ 撤销；commit 等只读源关闭
        let enable_selection = matches!(kind, GroupKind::Unstaged | GroupKind::Staged);
        let selected_clone = self.selected_diff_lines.clone();
        // render 期间 entity 已被 mut 借用，状态必须从 &self 读出后传给纯函数渲染器
        let has_blame = self.showing_blame && !self.blame_lines.is_empty();
        let expanded_spacers = self.expanded_diff_spacers.clone();
        super::diff_panel_split::render_file_diff_split(
            d,
            &selected_clone,
            enable_selection,
            false, // changes_only 已废弃，全部走「标准 / 全文件」二态后端控制
            mono,
            fg,
            muted_fg,
            muted_bg,
            &self.diff_scroll, // 两栏共享垂直 handle 保证行级同步
            &self.diff_h_scroll,
            &self.diff_h_scroll_right,
            has_blame,
            &expanded_spacers,
            cx,
        )
    }
}

fn placeholder(text: &'static str, muted_fg: gpui::Hsla) -> AnyElement {
    div()
        .px(px(12.0))
        .py(px(20.0))
        .text_sm()
        .text_color(muted_fg)
        .child(text)
        .into_any_element()
}

/// inline blame 顶部 banner：行号点击后显示该行的 commit / 作者 / 日期 / subject
/// 右侧 [×] 按钮关闭
fn render_inline_blame_banner(
    text: SharedString,
    accent: gpui::Hsla,
    fg: gpui::Hsla,
    mono: SharedString,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let mut chip_bg = accent;
    chip_bg.a = 0.10;
    h_flex()
        .w_full()
        .flex_none()
        .px(px(12.0))
        .py(px(4.0))
        .gap(px(8.0))
        .items_center()
        .bg(chip_bg)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .font_family(mono)
                .text_color(fg)
                .overflow_hidden()
                .text_ellipsis()
                .child(text),
        )
        .child(
            Button::new("vcs-inline-blame-close")
                .ghost()
                .xsmall()
                .icon(IconName::Close)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.clear_inline_blame(cx);
                })),
        )
        .into_any_element()
}
