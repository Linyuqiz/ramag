//! Key 树：SCAN 0→0 累计到 MAX_KEYS 后客户端过滤；按 `:` 折叠命名空间。
//! 同时是叶子+命名空间的节点（`user` 与 `user:1` 共存）单击仅展开，类型 badge 才加载值

mod render;
mod tree;

use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext as _, ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render,
    Styled, UniformListScrollHandle, Window, div, px, uniform_list,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use ramag_app::RedisService;
use ramag_domain::entities::{ConnectionConfig, KeyMeta};
use tracing::{error, info};

use tree::{TreeNode, VisibleRow, build_tree, collect_namespace_paths, has_match_descendant};

/// 单次最多加载的 key 数（防爆内存）
const MAX_KEYS: usize = 5_000;

/// 命名空间分隔符（业界事实标准）
const NAMESPACE_SEP: char = ':';

/// 单层缩进（像素）
pub(super) const INDENT_PX: f32 = 14.0;

#[derive(Debug, Clone)]
pub enum KeyTreeEvent {
    /// 用户选中某个 key
    Selected(String),
    /// 请求新建 Key（点击顶部 "+" 按钮）；由上层弹出 KeyCreateForm 对话框处理
    RequestCreate,
    /// 用户切换 DB（0-15）；由 Session 处理（同步详情/CLI/监控等子组件 + 重新加载树）
    DbSelected(u8),
}

pub struct KeyTreePanel {
    service: Arc<RedisService>,
    config: Option<ConnectionConfig>,
    db: u8,
    /// 已加载（缓存）的 key 列表（原始顺序）
    keys: Vec<KeyMeta>,
    /// 已加载 key 的 Trie 树（按 NAMESPACE_SEP 分层）
    tree: Vec<TreeNode>,
    /// 已展开的命名空间路径集合（按 full_path 索引）
    expanded: HashSet<String>,
    loading: bool,
    error: Option<String>,
    /// 客户端搜索框 / 关键字（小写）
    search: Entity<InputState>,
    query: String,
    /// 当前选中的 key（高亮）
    selected: Option<String>,
    /// 是否到达 MAX_KEYS 截断
    truncated: bool,
    /// 虚拟列表滚动句柄：树扁平化后用 uniform_list 行级虚拟化，
    /// 支持 5w+ key 仍流畅
    uniform_scroll: UniformListScrollHandle,
    _subscriptions: Vec<gpui::Subscription>,
}

impl EventEmitter<KeyTreeEvent> for KeyTreePanel {}

impl KeyTreePanel {
    pub fn new(service: Arc<RedisService>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("过滤 key（前缀 / 子串）"));

        let subs = vec![cx.subscribe_in(
            &search,
            window,
            |this: &mut Self, _, e: &InputEvent, _, cx| {
                if matches!(e, InputEvent::Change) {
                    this.query = this.search.read(cx).value().trim().to_lowercase();
                    cx.notify();
                }
            },
        )];

        Self {
            service,
            config: None,
            db: 0,
            keys: Vec::new(),
            tree: Vec::new(),
            expanded: HashSet::new(),
            loading: false,
            error: None,
            search,
            query: String::new(),
            selected: None,
            truncated: false,
            uniform_scroll: UniformListScrollHandle::new(),
            _subscriptions: subs,
        }
    }

    /// 切换连接 / DB → 重新拉一次 SCAN
    pub fn set_connection(
        &mut self,
        config: Option<ConnectionConfig>,
        db: u8,
        cx: &mut Context<Self>,
    ) {
        self.config = config;
        self.db = db;
        self.selected = None;
        self.error = None;
        self.keys.clear();
        self.tree.clear();
        self.expanded.clear();
        self.truncated = false;
        if self.config.is_some() {
            self.refresh(cx);
        } else {
            cx.notify();
        }
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        self.loading = true;
        self.error = None;
        cx.notify();

        let svc = self.service.clone();
        let db = self.db;
        cx.spawn(async move |this, cx| {
            let result = svc.scan_all(&config, db, None, None, MAX_KEYS).await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(keys) => {
                        info!(count = keys.len(), db, "redis scan_all completed");
                        this.truncated = keys.len() >= MAX_KEYS;
                        this.keys = keys;
                        this.rebuild_tree();
                    }
                    Err(e) => {
                        error!(error = %e, "redis scan_all failed");
                        this.error = Some(format!("加载失败：{e}"));
                        this.keys.clear();
                        this.tree.clear();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 由 keys 重建 Trie 树；默认展开第一层命名空间
    fn rebuild_tree(&mut self) {
        self.tree = build_tree(&self.keys);
        if self.expanded.is_empty() {
            for n in &self.tree {
                if n.is_namespace() {
                    self.expanded.insert(n.full_path.clone());
                }
            }
        }
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn db(&self) -> u8 {
        self.db
    }

    fn select_key(&mut self, key: String, cx: &mut Context<Self>) {
        self.selected = Some(key.clone());
        cx.emit(KeyTreeEvent::Selected(key));
        cx.notify();
    }

    /// 外部触发选中（如新建 Key 后由 Session 调用）
    pub fn select_key_external(&mut self, key: String, cx: &mut Context<Self>) {
        self.selected = Some(key.clone());
        cx.emit(KeyTreeEvent::Selected(key));
        cx.notify();
    }

    fn toggle_expanded(&mut self, path: String, cx: &mut Context<Self>) {
        if !self.expanded.remove(&path) {
            self.expanded.insert(path);
        }
        cx.notify();
    }

    fn matches_query(&self, key: &str) -> bool {
        if self.query.is_empty() {
            return true;
        }
        key.to_lowercase().contains(&self.query)
    }

    /// 把树扁平化为可见行列表（owned 结构，避免与 cx.listener 借用冲突）
    fn flatten_visible(&self) -> Vec<VisibleRow> {
        let mut out = Vec::new();
        let in_search = !self.query.is_empty();
        for n in &self.tree {
            self.collect_visible(n, 0, in_search, &mut out);
        }
        out
    }

    fn collect_visible(
        &self,
        node: &TreeNode,
        depth: usize,
        in_search: bool,
        out: &mut Vec<VisibleRow>,
    ) {
        let leaf_match = node.leaf_type.is_some() && self.matches_query(&node.full_path);
        let descendant_match = node.is_namespace() && has_match_descendant(node, &self.query);

        if in_search && !leaf_match && !descendant_match {
            return;
        }

        let is_namespace = node.is_namespace();
        let is_expanded = if in_search {
            descendant_match
        } else {
            self.expanded.contains(&node.full_path)
        };

        out.push(VisibleRow {
            depth,
            label: node.label.clone(),
            full_path: node.full_path.clone(),
            leaf_type: node.leaf_type,
            is_namespace,
            is_expanded,
        });

        if is_namespace && is_expanded {
            for c in &node.children {
                self.collect_visible(c, depth + 1, in_search, out);
            }
        }
    }

    fn expand_all(&mut self, cx: &mut Context<Self>) {
        self.expanded.clear();
        for n in &self.tree {
            collect_namespace_paths(n, &mut self.expanded);
        }
        cx.notify();
    }

    fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.expanded.clear();
        cx.notify();
    }
}

impl Render for KeyTreePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let border = theme.border;
        let bg = theme.background;
        let row_hover = theme.muted;
        let accent = theme.accent;

        let total = self.keys.len();
        let in_search = !self.query.is_empty();
        let visible = self.flatten_visible();
        let visible_leaf_count = visible.iter().filter(|r| r.leaf_type.is_some()).count();
        let selected = self.selected.clone();

        let count_label = if self.config.is_none() {
            "尚未连接".to_string()
        } else if self.loading {
            "加载中...".to_string()
        } else if let Some(ref e) = self.error {
            e.clone()
        } else if !in_search {
            format!(
                "共 {total} 个 key{}",
                if self.truncated {
                    "（已截断）"
                } else {
                    ""
                }
            )
        } else {
            format!("匹配 {visible_leaf_count} / {total}")
        };

        // 顶部第 1 行：DB 选择
        let current_db = self.db;
        let session_entity = cx.entity();
        let db_picker_label = format!("DB {current_db} ▾");
        let db_row = h_flex()
            .w_full()
            .px(px(10.0))
            .py(px(6.0))
            .border_b_1()
            .border_color(border)
            .gap(px(8.0))
            .items_center()
            .child(
                Button::new("kt-db-picker")
                    .ghost()
                    .small()
                    .label(db_picker_label)
                    .dropdown_menu_with_anchor(gpui::Anchor::BottomLeft, move |menu, _, _| {
                        let mut m = menu;
                        let entity = session_entity.clone();
                        for db in 0u8..=15 {
                            let is_active = db == current_db;
                            let label = if is_active {
                                format!("✓ DB {db}")
                            } else {
                                format!("  DB {db}")
                            };
                            let entity = entity.clone();
                            m = m.item(PopupMenuItem::new(label).on_click(move |_, _, app| {
                                entity.update(app, |this, cx| {
                                    if this.db != db {
                                        cx.emit(KeyTreeEvent::DbSelected(db));
                                    }
                                });
                            }));
                        }
                        m
                    }),
            );

        // 顶部第 2 行：搜索 + 新建 Key + 全展开 / 全折叠 / 刷新
        let header = h_flex()
            .w_full()
            .px(px(10.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(border)
            .gap(px(6.0))
            .items_center()
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.search)
                        .small()
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).small().text_color(muted_fg)),
                ),
            )
            .child(
                Button::new("redis-key-create")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Plus)
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                        cx.emit(KeyTreeEvent::RequestCreate);
                    })),
            )
            .child({
                let any_expanded = !self.expanded.is_empty();
                let (icon, tip) = if any_expanded {
                    (IconName::FolderOpen, "全部折叠命名空间")
                } else {
                    (IconName::FolderClosed, "全部展开命名空间")
                };
                Button::new("redis-key-toggle-all")
                    .ghost()
                    .xsmall()
                    .icon(icon)
                    .tooltip(tip)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if any_expanded {
                            this.collapse_all(cx);
                        } else {
                            this.expand_all(cx);
                        }
                    }))
            })
            .child(
                Button::new("redis-key-refresh")
                    .ghost()
                    .xsmall()
                    .icon(ramag_ui::icons::refresh_cw())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.refresh(cx))),
            );

        let theme_bg = theme.background;
        let theme_muted = theme.muted;

        // 树形渲染：扁平化为 Vec<VisibleRow>，喂给 uniform_list 行级虚拟化
        let visible_rc: Rc<Vec<VisibleRow>> = Rc::new(visible);
        let row_count = visible_rc.len();

        let empty_hint =
            !self.loading && total == 0 && self.config.is_some() && self.error.is_none();

        let body: gpui::AnyElement = if row_count == 0 {
            if empty_hint {
                div()
                    .flex_1()
                    .min_h_0()
                    .py(px(28.0))
                    .text_center()
                    .text_sm()
                    .text_color(muted_fg)
                    .child("DB 内没有 key")
                    .into_any_element()
            } else {
                div().flex_1().min_h_0().into_any_element()
            }
        } else {
            let visible_for_closure = visible_rc.clone();
            let selected_for_closure = selected.clone();
            uniform_list(
                "redis-key-tree-rows",
                row_count,
                cx.processor(move |this, range: Range<usize>, _w, cx| {
                    range
                        .map(|i| {
                            let row_data = &visible_for_closure[i];
                            this.render_node_row(
                                row_data,
                                &selected_for_closure,
                                fg,
                                muted_fg,
                                row_hover,
                                accent,
                                theme_bg,
                                theme_muted,
                                cx,
                            )
                            .into_any_element()
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.uniform_scroll)
            .flex_1()
            .into_any_element()
        };

        let status_bar = div()
            .flex_none()
            .w_full()
            .px(px(10.0))
            .py(px(4.0))
            .border_t_1()
            .border_color(border)
            .text_xs()
            .text_color(muted_fg)
            .child(count_label);

        v_flex()
            .size_full()
            .bg(bg)
            .child(db_row)
            .child(header)
            .child(body)
            .child(status_bar)
    }
}
