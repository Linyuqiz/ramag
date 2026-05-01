//! 仓库管理页（仿 dbclient 的 ConnectionListPanel）
//!
//! 布局：
//! ```text
//! ┌────────────────────────────────────────────────────────────────────┐
//! │     [🔍 搜索仓库（名称 / 路径）]                  [+ 选择本地仓库]   │  ← 1080px 居中
//! ├────────────────────────────────────────────────────────────────────┤
//! │  [Git]  Coding              /Users/.../Coding         [✏][🗑]      │
//! │  [Git]  ramag               /Users/.../Action/ramag   [✏][🗑]      │
//! └────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! 视觉对齐 [`super::super::super::ramag_tool_dbclient::views::connection_list`]：
//! - 内容区 1080px 居中（避免大屏摊太开）
//! - 行：类型 badge 76px / 名称 flex_1 加粗 / 路径 muted / 右侧编辑删除按钮
//! - 整行点击 = 打开仓库；操作按钮独立 emit（mouse_down 拦冒泡）
//! - 空态：大圆角块 + 主按钮「选择本地仓库」

use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, IntoElement, ParentElement, SharedString, Styled,
    div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    scroll::ScrollableElement as _,
    v_flex,
};

impl VcsView {
    /// 渲染顶部错误 banner（仅当 self.error 不空时返回 Some）
    ///
    /// 红色背景 + 错误文案 + ✕ 关闭按钮；不阻塞下方主内容交互。
    /// RepoList / IDE 布局都用这个 banner，统一错误展示位置在视图顶部。
    pub(super) fn render_error_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let err = self.error.as_ref()?;
        let theme = cx.theme();
        let mut banner_bg = theme.danger;
        banner_bg.a = 0.10;
        let danger = theme.danger;
        Some(
            h_flex()
                .w_full()
                .items_start()
                .gap(px(8.0))
                .px(px(16.0))
                .py(px(10.0))
                .bg(banner_bg)
                .border_b_1()
                .border_color(danger)
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .small()
                        .text_color(danger),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(danger)
                        .child(err.clone()),
                )
                .child(
                    Button::new("vcs-error-clear")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .tooltip("关闭错误提示")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.clear_error(cx);
                        })),
                )
                .into_any_element(),
        )
    }
}
use ramag_domain::entities::RepoConfig;

use super::vcs_view::VcsView;

/// 内容区最大宽度（与 dbclient connection_list 保持一致）
const CONTENT_MAX_W: f32 = 1080.0;

impl VcsView {
    /// 仓库管理页主入口（active_view == RepoList 时由 Render 路由调用）
    pub(super) fn render_repo_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let accent = theme.accent;
        let border = theme.border;
        let row_hover = theme.muted;
        let bg = theme.background;
        let busy = self.busy;

        // 当前搜索关键字（小写）；空 = 不过滤
        let query = self
            .repo_search_input
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let total = self.recent_repos.len();
        let filtered: Vec<&RepoConfig> = if query.is_empty() {
            self.recent_repos.iter().collect()
        } else {
            self.recent_repos
                .iter()
                .filter(|r| {
                    r.name.to_lowercase().contains(&query) || r.path.to_lowercase().contains(&query)
                })
                .collect()
        };
        let visible_count = filtered.len();

        // ===== Header =====
        // 极简布局：左侧搜索框（max 360px）+ 右侧 outline+small 「选择本地仓库」
        let header_inner = h_flex()
            .w_full()
            .items_center()
            .gap(px(16.0))
            .child(
                div().flex_1().min_w_0().child(
                    div().max_w(px(360.0)).child(
                        Input::new(&self.repo_search_input)
                            .small()
                            .cleanable(true)
                            .prefix(Icon::new(IconName::Search).small().text_color(muted_fg)),
                    ),
                ),
            )
            .child(
                Button::new("vcs-repo-clone")
                    .ghost()
                    .small()
                    .icon(ramag_ui::icons::download())
                    .tooltip("Clone 远程仓库到本地")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.show_clone_panel = !this.show_clone_panel;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("vcs-repo-init")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .tooltip("初始化新 Git 仓库")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        let dialog = rfd::FileDialog::new().set_title("选择或新建仓库目录");
                        if let Some(path) = dialog.pick_folder() {
                            this.init_repo_async(path, cx);
                        }
                    })),
            )
            .child(
                Button::new("vcs-repo-add")
                    .ghost()
                    .small()
                    .icon(IconName::FolderOpen)
                    .tooltip("打开本地 Git 仓库")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.pick_directory(cx);
                    })),
            );

        // 顶部和 tab bar 之间留呼吸空间（pt 比 pb 略大）
        let header = h_flex()
            .w_full()
            .justify_center()
            .px(px(24.0))
            .pt(px(22.0))
            .pb(px(16.0))
            .border_b_1()
            .border_color(border)
            .child(div().w_full().max_w(px(CONTENT_MAX_W)).child(header_inner));

        // ===== Body =====
        let body: AnyElement = if total == 0 {
            empty_state(border, muted_fg, fg, accent, cx)
        } else if visible_count == 0 {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .child(
                    div()
                        .text_sm()
                        .text_color(fg)
                        .child(format!("没有匹配「{query}」的仓库")),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted_fg)
                        .child("尝试修改关键字或清空搜索"),
                )
                .into_any_element()
        } else {
            let mut rows: Vec<AnyElement> = Vec::with_capacity(visible_count);
            for (idx, r) in filtered.into_iter().enumerate() {
                rows.push(
                    repo_row(idx, r, busy, border, row_hover, accent, fg, muted_fg, cx)
                        .into_any_element(),
                );
            }
            v_flex()
                .size_full()
                .overflow_y_scrollbar()
                .child(
                    h_flex()
                        .w_full()
                        .justify_center()
                        .px(px(24.0))
                        .py(px(10.0))
                        .child(v_flex().w_full().max_w(px(CONTENT_MAX_W)).children(rows)),
                )
                .into_any_element()
        };

        let mut root = v_flex().size_full().bg(bg);
        if let Some(banner) = self.render_error_banner(cx) {
            root = root.child(banner);
        }
        root.child(header)
            .when(self.show_clone_panel, |c| {
                c.child(self.render_clone_panel(cx))
            })
            .child(body)
            .into_any_element()
    }

    /// Clone 面板（`show_clone_panel = true` 时在 header 下方内联展示）
    fn render_clone_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let border = theme.border;
        let bg = theme.background;
        let busy = self.loading;
        let dest_label = self
            .clone_dest_path
            .as_ref()
            .and_then(|p| p.to_str().map(str::to_string))
            .unwrap_or_else(|| "点击选择目标目录".into());

        h_flex()
            .w_full()
            .items_center()
            .gap(px(8.0))
            .px(px(24.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(border)
            .bg(bg)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(gpui_component::input::Input::new(&self.clone_url_input).small()),
            )
            .child(
                Button::new("vcs-clone-pick-dest")
                    .ghost()
                    .small()
                    .icon(IconName::Folder)
                    .label(dest_label)
                    .tooltip("选择 Clone 目标目录")
                    .disabled(busy)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        let dialog = rfd::FileDialog::new().set_title("选择 Clone 目标目录");
                        if let Some(path) = dialog.pick_folder() {
                            this.clone_dest_path = Some(path);
                            cx.notify();
                        }
                    })),
            )
            .child(
                Button::new("vcs-clone-execute")
                    .primary()
                    .small()
                    .icon(ramag_ui::icons::download())
                    .label("Clone")
                    .disabled(busy || self.clone_dest_path.is_none())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        let url = this.clone_url_input.read(cx).value().trim().to_string();
                        if url.is_empty() {
                            this.error = Some("请输入仓库 URL".into());
                            cx.notify();
                            return;
                        }
                        let Some(dest) = this.clone_dest_path.clone() else {
                            return;
                        };
                        // dest = parent_dir / repo_name derived from url
                        let repo_name = url
                            .split('/')
                            .next_back()
                            .unwrap_or("repo")
                            .trim_end_matches(".git")
                            .to_string();
                        let dest_full = dest.join(repo_name);
                        this.clone_repo_async(url, dest_full, cx);
                    })),
            )
            .child(
                Button::new("vcs-clone-cancel")
                    .ghost()
                    .small()
                    .label("取消")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.show_clone_panel = false;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
}

/// 单条仓库行（整行点击 = 打开；行内删除按钮独立 emit）
#[allow(clippy::too_many_arguments)]
fn repo_row(
    idx: usize,
    r: &RepoConfig,
    busy: bool,
    border: gpui::Hsla,
    hover_bg: gpui::Hsla,
    accent: gpui::Hsla,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    cx: &mut Context<VcsView>,
) -> impl IntoElement {
    // Git 类型 badge（与 dbclient driver 一类一色对齐：Git 用 accent 蓝）
    let badge_fg = accent;
    let mut badge_bg = badge_fg;
    badge_bg.a = 0.12;

    let path_for_open = r.path.clone();
    let path_for_remove = r.path.clone();
    let row_id = SharedString::from(format!("vcs-repo-row-{idx}-{}", r.path));
    let del_id = SharedString::from(format!("vcs-repo-del-{idx}-{}", r.path));

    let mono = cx.theme().mono_font_family.clone();

    h_flex()
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
            this.open_recent_repo(path_for_open.clone(), cx);
        }))
        // 类型 badge（76px 与 dbclient 对齐）
        .child(
            div().flex_none().w(px(76.0)).flex().justify_center().child(
                div()
                    .px(px(8.0))
                    .py(px(2.0))
                    .rounded(px(4.0))
                    .text_xs()
                    .text_color(badge_fg)
                    .bg(badge_bg)
                    .child("Git"),
            ),
        )
        // 名称（最重要，flex_1 占主空间，加粗）
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg)
                .overflow_hidden()
                .text_ellipsis()
                .child(r.name.clone()),
        )
        // 路径（mono 小灰，尾部省略；右对齐占据 360px）
        .child(
            div()
                .flex_none()
                .w(px(360.0))
                .text_xs()
                .text_color(muted_fg)
                .font_family(mono)
                .overflow_hidden()
                .text_ellipsis()
                .child(r.path.clone()),
        )
        // 操作按钮组（80px 右对齐，mouse_down 拦冒泡避免触发整行打开）
        .child(
            h_flex()
                .flex_none()
                .gap(px(4.0))
                .w(px(80.0))
                .justify_end()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    Button::new(del_id)
                        .ghost()
                        .small()
                        .icon(ramag_ui::icons::trash())
                        .tooltip("从最近列表移除（不删除磁盘文件）")
                        .disabled(busy)
                        // 弹确认对话框（与 dbclient 删除连接同款交互），用户确认后再真正移除
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.confirm_remove_recent_repo(path_for_remove.clone(), window, cx);
                        })),
                ),
        )
}

/// 空状态：大圆角块 + 主按钮「选择本地仓库」
fn empty_state(
    border: gpui::Hsla,
    muted_fg: gpui::Hsla,
    fg: gpui::Hsla,
    accent: gpui::Hsla,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let mut tinted_accent = accent;
    tinted_accent.a = 0.12;

    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap(px(20.0))
        .child(
            div()
                .w(px(64.0))
                .h(px(64.0))
                .rounded(px(14.0))
                .bg(tinted_accent)
                .flex()
                .items_center()
                .justify_center()
                .child(ramag_ui::icons::git_branch().text_color(accent)),
        )
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(fg)
                .child("还没打开过 Git 仓库"),
        )
        .child(
            div()
                .text_sm()
                .text_color(muted_fg)
                .child("点击下方按钮选择第一个本地 Git 仓库目录"),
        )
        .child(
            Button::new("vcs-repo-empty-pick")
                .primary()
                .icon(IconName::Plus)
                .label("打开仓库")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.pick_directory(cx);
                })),
        )
        .pb(px(64.0))
        .pt(px(64.0))
        .mx(px(40.0))
        .border_1()
        .border_color(border)
        .rounded_lg()
        .into_any_element()
}
