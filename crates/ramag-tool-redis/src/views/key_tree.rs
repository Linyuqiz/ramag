//! Key 树面板：SCAN 分批 + 命名空间分组
//!
//! Stage 16 升级：按 `:` 分隔的折叠命名空间树
//! - SCAN 0→0 完整迭代，每次 200 个，最多累计 MAX_KEYS（防爆内存）
//! - 顶部搜索框 → 客户端过滤已加载 keys（搜索时自动展开匹配命名空间）
//! - 树形结构：中间节点 ▶/▼ 折叠、叶子单击加载值
//! - 类型徽标 + 配色（zedis / RedisInsight 同款）
//!
//! 已知约束：
//! - 仅支持单字符分隔符 `:`（业界事实标准）
//! - 同名 key 与命名空间冲突时（罕见，如 `user` 既是 key 又是 `user:1` 的前缀），
//!   该节点同时是叶子+命名空间，单击仅展开；点击右侧类型 badge 加载值

use std::collections::HashSet;
use std::sync::Arc;

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    scroll::ScrollableElement as _,
    v_flex,
};
use ramag_app::RedisService;
use ramag_domain::entities::{ConnectionConfig, KeyMeta, RedisType};
use tracing::{error, info};

/// 单次最多加载的 key 数（防爆内存）
const MAX_KEYS: usize = 5_000;

/// 命名空间分隔符（业界事实标准）
const NAMESPACE_SEP: char = ':';

/// 单层缩进（像素）
const INDENT_PX: f32 = 14.0;

#[derive(Debug, Clone)]
pub enum KeyTreeEvent {
    /// 用户选中某个 key
    Selected(String),
    /// 请求新建 Key（点击顶部 "+" 按钮）；由上层弹出 KeyCreateForm 对话框处理
    RequestCreate,
    /// 用户切换 DB（0-15）；由 Session 处理（同步详情/CLI/监控等子组件 + 重新加载树）
    DbSelected(u8),
}

/// 树节点：可同时是命名空间（有子节点）和叶子（对应实际 key）
#[derive(Debug, Clone)]
struct TreeNode {
    /// 当前层显示标签（路径中的一段）
    label: String,
    /// 完整路径（叶子时是完整 key 名；中间节点是路径前缀）
    full_path: String,
    /// 子节点（按 label 排序：命名空间在前，叶子在后；同类按字母升序）
    children: Vec<TreeNode>,
    /// 该节点本身是否对应实际 key（叶子状态；可同时有 children）
    leaf_type: Option<RedisType>,
}

impl TreeNode {
    fn is_namespace(&self) -> bool {
        !self.children.is_empty()
    }
}

/// 渲染层用的扁平行（拥有数据，避免与 cx.listener 借用冲突）
#[derive(Debug, Clone)]
struct VisibleRow {
    depth: usize,
    label: String,
    full_path: String,
    leaf_type: Option<RedisType>,
    is_namespace: bool,
    is_expanded: bool,
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

    /// 由 keys 重建 Trie 树；默认展开第一层命名空间，让用户看到结构
    fn rebuild_tree(&mut self) {
        self.tree = build_tree(&self.keys);
        // 默认展开第一层命名空间（每个根级子树的第一层折叠）
        // 仅在初次构建时设；用户后续手动 toggle 不被覆盖
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

    /// 外部触发选中（如新建 Key 后由 Session 调用）：仅高亮，不再次 emit Selected
    /// 让 Session 同时调 detail.load_key 即可，避免重复加载值
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

    /// 搜索过滤后的 key（用于决定哪些命名空间需要在树视图中可见）
    fn matches_query(&self, key: &str) -> bool {
        if self.query.is_empty() {
            return true;
        }
        key.to_lowercase().contains(&self.query)
    }

    /// 把树扁平化为可见行列表（owned 结构，避免与 cx.listener 借用冲突）
    ///
    /// 搜索模式（query 非空）下：
    /// - 不应用 expanded 状态，强制展开所有有匹配后代的命名空间
    /// - 仅显示叶子匹配 query 的路径
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

        // 顶部第 1 行：DB 选择 + 总数（仿 MySQL 表树的 schema 节点 + 行数显示）
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
                                        // 仅 emit 事件让 Session 调度（包括关 KeyDetail tabs / 重载等）
                                        cx.emit(KeyTreeEvent::DbSelected(db));
                                    }
                                });
                            }));
                        }
                        m
                    }),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_xs()
                    .text_color(muted_fg)
                    .child(count_label.clone()),
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
            .child(
                Button::new("redis-key-expand-all")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronDown)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.expand_all(cx))),
            )
            .child(
                Button::new("redis-key-collapse-all")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronRight)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.collapse_all(cx))),
            )
            .child(
                Button::new("redis-key-refresh")
                    .ghost()
                    .xsmall()
                    .icon(ramag_ui::icons::refresh_cw())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.refresh(cx))),
            );

        // info_line 已并入 db_row 顶部，不再单独渲染（避免重复"共 N 个 key"）

        // 提前把 theme 里需要的颜色拷出来，让 theme 借用尽快释放
        // （render_node_row 内部 cx.listener 需要 &mut cx，与 &theme 冲突）
        let theme_bg = theme.background;
        let theme_muted = theme.muted;

        // 树形渲染：扁平化后逐行画
        let mut rows = v_flex().w_full().gap(px(0.0));
        for row_data in visible {
            rows = rows.child(self.render_node_row(
                &row_data,
                &selected,
                fg,
                muted_fg,
                row_hover,
                accent,
                theme_bg,
                theme_muted,
                cx,
            ));
        }

        if !self.loading && total == 0 && self.config.is_some() && self.error.is_none() {
            rows = rows.child(
                div()
                    .py(px(28.0))
                    .text_center()
                    .text_sm()
                    .text_color(muted_fg)
                    .child("DB 内没有 key"),
            );
        }

        v_flex()
            .size_full()
            .bg(bg)
            .child(db_row)
            .child(header)
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(rows),
            )
    }
}

impl KeyTreePanel {
    /// 渲染单行（命名空间或叶子）
    ///
    /// `+ use<>`：Rust 2024 默认捕获所有 lifetime，会让返回值绑死在 &self 上，
    /// 与同函数内 `cx.listener(...)` 需要的 `&mut Context<Self>` 借用冲突。
    /// 显式声明不捕获生命周期，确保返回值是 'static 风格
    #[allow(clippy::too_many_arguments)]
    fn render_node_row(
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

        let mut row_el = h_flex()
            .id(row_id)
            .w_full()
            .items_center()
            .gap(px(6.0))
            .pl(px(8.0 + row.depth as f32 * INDENT_PX))
            .pr(px(10.0))
            .py(px(3.0))
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

// ===== Trie 构建辅助 =====

fn build_tree(keys: &[KeyMeta]) -> Vec<TreeNode> {
    let mut roots: Vec<TreeNode> = Vec::new();
    for k in keys {
        let parts: Vec<&str> = k.key.split(NAMESPACE_SEP).collect();
        if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
            // 跳过空 key 或形如 "::" 的异常路径
            continue;
        }
        insert_path(&mut roots, &parts, 0, k.key.clone(), k.key_type);
    }
    sort_recursive(&mut roots);
    roots
}

fn insert_path(
    nodes: &mut Vec<TreeNode>,
    parts: &[&str],
    idx: usize,
    full_key: String,
    kind: Option<RedisType>,
) {
    let part = parts[idx];
    let is_last = idx == parts.len() - 1;
    let path_so_far = parts[..=idx].join(":");

    if let Some(p) = nodes.iter().position(|n| n.label == part) {
        if is_last {
            nodes[p].leaf_type = kind;
            nodes[p].full_path = full_key;
        } else {
            insert_path(&mut nodes[p].children, parts, idx + 1, full_key, kind);
        }
    } else {
        let mut new_node = TreeNode {
            label: part.to_string(),
            full_path: path_so_far,
            children: Vec::new(),
            leaf_type: None,
        };
        if is_last {
            new_node.full_path = full_key;
            new_node.leaf_type = kind;
        } else {
            insert_path(&mut new_node.children, parts, idx + 1, full_key, kind);
        }
        nodes.push(new_node);
    }
}

fn sort_recursive(nodes: &mut [TreeNode]) {
    nodes.sort_by(|a, b| {
        // 命名空间在前，叶子在后；同类按 label 升序
        match (a.is_namespace(), b.is_namespace()) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.label.cmp(&b.label),
        }
    });
    for n in nodes {
        sort_recursive(&mut n.children);
    }
}

/// 在搜索模式下：判断节点的子树里是否有匹配 query 的叶子
fn has_match_descendant(node: &TreeNode, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    if node.leaf_type.is_some() && node.full_path.to_lowercase().contains(query) {
        return true;
    }
    for c in &node.children {
        if c.full_path.to_lowercase().contains(query) || has_match_descendant(c, query) {
            return true;
        }
    }
    false
}

fn collect_namespace_paths(node: &TreeNode, out: &mut HashSet<String>) {
    if node.is_namespace() {
        out.insert(node.full_path.clone());
        for c in &node.children {
            collect_namespace_paths(c, out);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(key: &str, t: RedisType) -> KeyMeta {
        KeyMeta {
            key: key.to_string(),
            key_type: Some(t),
            ttl_ms: None,
        }
    }

    #[test]
    fn build_simple_tree() {
        let keys = vec![
            meta("user:1:profile", RedisType::Hash),
            meta("user:2:profile", RedisType::Hash),
            meta("session:abc", RedisType::String),
        ];
        let tree = build_tree(&keys);
        // 命名空间在前（session 与 user 都是命名空间）
        assert!(tree.iter().all(|n| n.is_namespace()));
        let labels: Vec<_> = tree.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, vec!["session", "user"]);
    }

    #[test]
    fn leaf_and_namespace_coexist() {
        // user 既是 key（"user"）也是命名空间（"user:1"）
        let keys = vec![
            meta("user", RedisType::String),
            meta("user:1", RedisType::Hash),
        ];
        let tree = build_tree(&keys);
        assert_eq!(tree.len(), 1);
        let user_node = &tree[0];
        assert_eq!(user_node.label, "user");
        assert!(user_node.leaf_type.is_some());
        assert_eq!(user_node.children.len(), 1);
        assert_eq!(user_node.children[0].label, "1");
    }

    #[test]
    fn skip_empty_segments() {
        let keys = vec![
            meta("good:key", RedisType::String),
            meta("::bad", RedisType::String),
        ];
        let tree = build_tree(&keys);
        let labels: Vec<_> = tree.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, vec!["good"]);
    }

    #[test]
    fn search_descendant_match() {
        let keys = vec![meta("user:1:profile", RedisType::Hash)];
        let tree = build_tree(&keys);
        assert!(has_match_descendant(&tree[0], "profile"));
        assert!(has_match_descendant(&tree[0], "1"));
        assert!(!has_match_descendant(&tree[0], "session"));
    }

    #[test]
    fn collect_paths() {
        let keys = vec![
            meta("a:b:c", RedisType::String),
            meta("a:d", RedisType::Set),
        ];
        let tree = build_tree(&keys);
        let mut paths = HashSet::new();
        for n in &tree {
            collect_namespace_paths(n, &mut paths);
        }
        // "a" 和 "a:b" 是命名空间；"a:b:c" 和 "a:d" 是叶子
        assert!(paths.contains("a"));
        assert!(paths.contains("a:b"));
        assert!(!paths.contains("a:b:c"));
        assert!(!paths.contains("a:d"));
    }
}
