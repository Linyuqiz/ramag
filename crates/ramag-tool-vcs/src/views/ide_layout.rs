//! IDE 风布局：toolbar + op banner + 上半（左 files / 右 main）+ 下半 history。
//! 拖拽：`ide_main_resize` 上下 / `ide_left_resize` 上半左右；侧栏靠 toggle 切换，固定 220px

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, ParentElement, Styled, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    resizable::{h_resizable, resizable_panel, v_resizable},
    scroll::ScrollableElement as _,
    v_flex,
};

use super::helpers::FilesViewMode;
use super::vcs_view::VcsView;

/// 上半区初始高度（上半 = 工作区 + diff；下半 = history）
const TOP_HEIGHT_INITIAL: f32 = 600.0;
const TOP_HEIGHT_MIN: f32 = 200.0;
const TOP_HEIGHT_MAX: f32 = 1400.0;
/// 上半区内左侧 Files+Commit panel 初始宽度（默认按最小，避免左栏抢主区宽度）
const LEFT_WIDTH_INITIAL: f32 = 280.0;
const LEFT_WIDTH_MIN: f32 = 220.0;
const LEFT_WIDTH_MAX: f32 = 600.0;

impl VcsView {
    /// IDE 布局主入口：v_resizable{ 上半 h_resizable, 下半 history（可选） }
    pub(super) fn render_ide_layout(&self, cx: &mut Context<Self>) -> AnyElement {
        let row = h_flex().size_full().min_h_0();
        // 顶部错误 banner（self.error 不空时显示，可手动关闭）
        // 与 op_banner（merge/rebase 进行中横幅）并列；不独占 body 不阻塞操作
        let mut main_layout = v_flex().size_full();
        if let Some(banner) = self.render_error_banner(cx) {
            main_layout = main_layout.child(banner);
        }
        // 历史 pane 默认隐藏：仅渲染上半区独占整屏；用户从工具栏 toggle 才出现
        let main_layout = main_layout.child(self.render_op_banner(cx));
        let main_layout = if self.history_pane_visible {
            main_layout.child(
                v_resizable("vcs-ide-main")
                    .with_state(&self.ide_files_resize)
                    .child(
                        resizable_panel()
                            .size(px(TOP_HEIGHT_INITIAL))
                            .size_range(px(TOP_HEIGHT_MIN)..px(TOP_HEIGHT_MAX))
                            .child(div().size_full().child(self.render_top_pane(cx))),
                    )
                    .child(
                        resizable_panel()
                            .child(div().size_full().child(self.render_history_pane(cx))),
                    ),
            )
        } else {
            main_layout.child(div().flex_1().min_h_0().child(self.render_top_pane(cx)))
        };

        row.child(div().flex_1().min_w_0().h_full().child(main_layout))
            .into_any_element()
    }

    /// 上半区：左 files + commit panel / 右 diff or commit_detail
    fn render_top_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let border = theme.border;
        h_resizable("vcs-ide-top")
            .with_state(&self.ide_left_resize)
            .child(
                resizable_panel()
                    .size(px(LEFT_WIDTH_INITIAL))
                    .size_range(px(LEFT_WIDTH_MIN)..px(LEFT_WIDTH_MAX))
                    .child(
                        div()
                            .size_full()
                            .border_r_1()
                            .border_color(border)
                            .child(self.render_files_pane(cx)),
                    ),
            )
            .child(
                resizable_panel()
                    .child(div().size_full().min_w_0().child(self.render_main_area(cx))),
            )
            .into_any_element()
    }

    /// 上半左侧「Files」：tabs/搜索 + 内容区 + commit panel
    /// 仿 IDEA Git Tool Window：默认 Project Files；分支徽标 / Git 操作菜单都在 toolbar 内
    fn render_files_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .size_full()
            .child(self.render_files_toolbar(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px(px(10.0))
                    .py(px(6.0))
                    .overflow_y_scrollbar()
                    .child(self.render_files_content(cx)),
            )
            // commit 面板仅在 Changes 视图下显示（其他模式下隐藏）
            .when(
                matches!(self.files_view_mode, FilesViewMode::Changes),
                |c| c.child(self.render_commit_panel(cx)),
            )
            .into_any_element()
    }

    /// Files panel 顶部工具栏：[mode tabs（一排图标）] + 搜索框 + 刷新按钮
    /// tabs 用 segmented icon button 风格：4 个图标横排，选中态 selected() 高亮
    fn render_files_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let border = theme.border;
        let muted_fg = theme.muted_foreground;
        let busy = self.busy;
        let active = self.files_view_mode;

        // 第 1 行：3 个 segmented icon tabs（项目文件 / 本地变更 / 暂存）
        // 分支管理通过分支选择器 + 历史面板左栏操作，不再占用独立 tab
        let modes = [
            FilesViewMode::Project,
            FilesViewMode::Changes,
            FilesViewMode::Stash,
        ];
        let mut tabs_row = h_flex().gap(px(2.0)).items_center();
        for mode in modes {
            tabs_row = tabs_row.child(self.mode_tab_button(mode, active, cx));
        }
        // 末尾：分支选择器（显示当前 HEAD 分支名 + dropdown 列出所有分支可切换 / 创建）
        let mode_row = h_flex()
            .w_full()
            .px(px(10.0))
            .py(px(6.0))
            .border_b_1()
            .border_color(border)
            .gap(px(8.0))
            .items_center()
            .child(tabs_row)
            .child(div().flex_1())
            .child(self.render_branch_picker(cx));

        // 第 2 行：搜索框 + (Project 模式才显示的)全展开/全折叠 toggle + 刷新按钮
        // toggle 单按钮模式与 redis key_tree 对齐：根据当前是否有展开目录决定图标和动作
        let mut search_row = h_flex()
            .w_full()
            .items_center()
            .px(px(10.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(border)
            .gap(px(6.0))
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.files_search_input)
                        .small()
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).small().text_color(muted_fg)),
                ),
            );
        if matches!(active, FilesViewMode::Project) {
            let any_expanded = !self.project_expanded_dirs.is_empty();
            let (icon, tip) = if any_expanded {
                (IconName::FolderOpen, "全部折叠目录")
            } else {
                (IconName::FolderClosed, "全部展开目录")
            };
            search_row = search_row.child(
                Button::new("vcs-pf-toggle-all")
                    .ghost()
                    .xsmall()
                    .icon(icon)
                    .tooltip(tip)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if any_expanded {
                            this.collapse_all_project_dirs(cx);
                        } else {
                            this.expand_all_project_dirs(cx);
                        }
                    })),
            );
        }
        // 历史面板 toggle：从 ⋮ 菜单提出来的独立按钮（与展开/折叠并列）
        // 仅 repo 已打开时显示——RepoList 模式不需要
        if self.repo.is_some() {
            let history_visible = self.history_pane_visible;
            search_row = search_row.child(
                Button::new("vcs-history-pane-toggle")
                    .ghost()
                    .xsmall()
                    .icon(if history_visible {
                        IconName::PanelBottom
                    } else {
                        IconName::PanelBottomOpen
                    })
                    .tooltip(if history_visible {
                        "隐藏历史 / Reflog 面板"
                    } else {
                        "显示历史 / Reflog 面板"
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.toggle_history_pane(cx);
                    })),
            );
        }
        // 末尾：Git 操作聚合菜单（Fetch / Pull / Push / 强推）
        search_row = search_row.child(self.render_remote_actions(cx));
        let _ = busy; // 暂留：日后若禁用按钮可用

        v_flex()
            .child(mode_row)
            .child(search_row)
            .into_any_element()
    }

    /// 根据当前 mode 渲染 Files panel 内容区
    fn render_files_content(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.files_view_mode {
            FilesViewMode::Changes => self.render_file_groups(cx),
            FilesViewMode::Project => self.render_project_files_view(cx),
            FilesViewMode::Stash => self.render_stash_view(cx),
        }
    }

    /// 当前 HEAD 分支按钮 + dropdown：新建分支 / 本地 / 远程
    fn render_branch_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.repo.is_none() {
            return div().into_any_element();
        }
        let theme = cx.theme();
        let accent = theme.accent;
        let head = self
            .status
            .as_ref()
            .and_then(|s| s.head_branch.clone())
            .unwrap_or_else(|| "(detached)".into());
        let label = format!("{head} ▾");
        let busy = self.busy;
        let entity = cx.entity();
        let local: Vec<(String, bool, Option<String>)> = self
            .local_branches
            .iter()
            .map(|b| {
                let sync = match (b.ahead, b.behind) {
                    (Some(a), Some(d)) if a > 0 || d > 0 => Some(format!("↑{a} ↓{d}")),
                    _ => None,
                };
                (b.name.clone(), b.is_head, sync)
            })
            .collect();
        let remote: Vec<String> = self
            .remote_branches
            .iter()
            .map(|b| b.name.clone())
            .collect();
        // 加边框 + 深色文字，比纯 ghost+蓝字更醒目（用户反馈纯文字辨识度低）
        let _ = accent;
        Button::new("vcs-branch-picker")
            .outline()
            .small()
            .label(label)
            .text_color(cx.theme().foreground)
            .tooltip("切换分支 / 创建分支")
            .disabled(busy)
            .dropdown_menu_with_anchor(
                gpui::Anchor::BottomRight,
                move |mut m: PopupMenu, window, cx| {
                    // 父菜单不能 scrollable —— 否则 submenu 不工作（gpui-component 限制）。
                    // 分支用 group 收纳后顶层项数量可控，通常窗口能装下；
                    // 多分支的 prefix（如 origin/*）放进各自 submenu，submenu 自身可 scrollable
                    // max_w 限宽防止超长分支名撑破菜单（叶子内部已做中间省略截断）
                    m = m.max_w(px(420.0));
                    // 操作分组
                    m = m.item(PopupMenuItem::label("操作"));
                    let ent_new = entity.clone();
                    let head_for_dlg = local
                        .iter()
                        .find(|(_, is_head, _)| *is_head)
                        .map(|(n, _, _)| n.clone())
                        .unwrap_or_else(|| "(HEAD)".into());
                    m = m.item(PopupMenuItem::new("✦ 新建分支...").on_click({
                        let ent = ent_new.clone();
                        let hdlg = head_for_dlg.clone();
                        let local_for_dlg = local
                            .iter()
                            .map(|(n, h, _)| (n.clone(), *h))
                            .collect::<Vec<_>>();
                        let remote_for_dlg = remote.clone();
                        move |_, window, app| {
                            super::branch_picker::open_new_branch_dialog(
                                ent.clone(),
                                hdlg.clone(),
                                local_for_dlg.clone(),
                                remote_for_dlg.clone(),
                                window,
                                app,
                            );
                        }
                    }));
                    m = m.separator();
                    // 本地分支：按 / 路径分组（feature/xxx → 「feature」 submenu，hover 展开侧菜单）
                    m = m.item(PopupMenuItem::label("本地"));
                    m = super::branch_picker::render_branches_grouped(
                        m,
                        &local,
                        false,
                        ent_new.clone(),
                        window,
                        cx,
                    );
                    // 远程分支：所有 origin/* 自动归入「origin」 submenu（同款 hover 侧栏）
                    if !remote.is_empty() {
                        m = m.separator();
                        m = m.item(PopupMenuItem::label("远程"));
                        let remote_items: Vec<(String, bool, Option<String>)> =
                            remote.iter().map(|n| (n.clone(), false, None)).collect();
                        m = super::branch_picker::render_branches_grouped(
                            m,
                            &remote_items,
                            true,
                            ent_new.clone(),
                            window,
                            cx,
                        );
                    }
                    m
                },
            )
            .into_any_element()
    }
}

impl VcsView {
    fn mode_tab_button(
        &self,
        mode: FilesViewMode,
        active: FilesViewMode,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = gpui::SharedString::from(format!("vcs-files-tab-{}", mode.id_str()));
        let is_active = mode == active;
        // mode 与图标的映射：folder 表项目，file 表本地变更，inbox 表 stash，git-branch 走 ramag-ui
        let mut btn = Button::new(id)
            .ghost()
            .small()
            .selected(is_active)
            .tooltip(mode.label());
        btn = match mode {
            FilesViewMode::Project => btn.icon(IconName::Folder),
            FilesViewMode::Changes => btn.icon(IconName::File),
            FilesViewMode::Stash => btn.icon(IconName::Inbox),
        };
        btn.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.set_files_view_mode(mode, cx);
        }))
        .into_any_element()
    }

    /// Stash 视图：在主 panel 显示完整 stash 列表（与左侧栏 Stash 段共用数据）
    fn render_stash_view(&self, cx: &mut Context<Self>) -> AnyElement {
        // 复用 sidebar 的 stash 段渲染：列表 + 行尾按钮（apply / pop / drop）
        // 给一个独立 wrapper，避免被 sidebar 的折叠样式影响
        v_flex()
            .size_full()
            .child(self.render_stash_list_body(cx))
            .into_any_element()
    }

    /// 下半区 History：commit 列表 + 搜索 / Reflog 切换。横跨右半（侧栏外）
    fn render_history_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        self.render_history_view(cx)
    }

    /// 优先级：rebase 计划 > 冲突编辑器 > file_tab（Changes / Commit / ProjectFiles 共用 render_diff_block）
    fn render_main_area(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.show_rebase_plan {
            return self.render_rebase_plan(cx);
        }
        if self.conflict_editor_path.is_some() {
            return self.render_conflict_editor(cx);
        }
        let is_pf_active = self
            .active_file_tab_idx
            .and_then(|i| self.file_tabs.get(i))
            .map(|t| matches!(t.source, super::helpers::FileTabSource::ProjectFiles))
            .unwrap_or(false);
        let body = if is_pf_active {
            self.render_pf_content(cx)
        } else {
            self.render_diff_block(cx)
        };
        v_flex()
            .size_full()
            .min_w_0()
            .child(self.render_file_tab_bar(cx))
            .child(div().flex_1().min_h_0().min_w_0().w_full().child(body))
            .into_any_element()
    }
}
