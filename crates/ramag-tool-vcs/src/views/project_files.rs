//! Project Files：git ls-files → 嵌套树 → ProjectRow → uniform_list 行级虚拟化（28px 等高）。
//! 默认全部折叠（IDE 习惯，避免一打开全展开）；状态字母色复用 `helpers::code_letter_color`

use std::collections::BTreeMap;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::*, px, uniform_list,
};
use gpui_component::{ActiveTheme, h_flex, v_flex};
use ramag_domain::entities::{FileChangeKind, FileStatus};

use super::helpers::{code_letter_color, code_to_letter};
use super::vcs_view::VcsView;

/// 树节点：目录（含子节点 BTreeMap，按名字字母序）或文件（叶子）
enum Node {
    /// 目录：children 按名字字母序，目录排前文件排后
    Dir(BTreeMap<String, Node>),
    /// 文件：相对仓库根的完整路径（用于 status 查找 / select_file）
    File { full_path: String },
}

/// 节点 map 类型别名：目录名 → 子节点；BTreeMap 自带字母序
type NodeMap = BTreeMap<String, Node>;

/// `split_dirs_files` 返回类型：(目录节点列表, 文件节点列表)
type SplitNodes = (Vec<(String, Node)>, Vec<(String, Node)>);

/// uniform_list 行单元，所有变体高度必须等于 28px
#[derive(Clone)]
pub(super) enum ProjectRow {
    /// 目录行：箭头 + 名字，可点击折叠/展开
    Dir {
        name: String,
        dir_path: String,
        depth: usize,
        is_expanded: bool,
    },
    /// 文件行：状态字母 + 名字，可点击查看 diff
    File {
        name: String,
        full_path: String,
        depth: usize,
    },
}

/// 缓存：三 key (files_version, expanded_version, query) 全等命中复用 rows
pub(super) struct ProjectRowsCacheEntry {
    pub rows: Rc<Vec<ProjectRow>>,
    pub files_version: u64,
    pub expanded_version: u64,
    pub query: String,
}

/// 把扁平 path 列表（已排序）构建成嵌套目录树
///
/// 例：`["a/b.rs", "a/c.rs", "d.rs"]` → `Dir { a: Dir { b.rs: File, c.rs: File }, d.rs: File }`
fn build_tree(paths: &[String]) -> NodeMap {
    let mut root: NodeMap = BTreeMap::new();
    for path in paths {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.is_empty() {
            continue;
        }
        insert_path(&mut root, &parts, path);
    }
    root
}

/// 把单条 path 的 parts 列表插入到树中
fn insert_path(map: &mut NodeMap, parts: &[&str], full_path: &str) {
    if parts.is_empty() {
        return;
    }
    let head = parts[0].to_string();
    if parts.len() == 1 {
        map.insert(
            head,
            Node::File {
                full_path: full_path.to_string(),
            },
        );
        return;
    }
    let entry = map
        .entry(head)
        .or_insert_with(|| Node::Dir(BTreeMap::new()));
    if let Node::Dir(children) = entry {
        insert_path(children, &parts[1..], full_path);
    }
}

/// DFS 扁平化。`expanded` 不含的目录视为折叠；每层先目录后文件、按 BTreeMap 字母序
fn flatten(
    map: NodeMap,
    expanded: &std::collections::HashSet<String>,
    parent_path: &str,
    depth: usize,
    out: &mut Vec<ProjectRow>,
) {
    let (dirs, files) = split_dirs_files(map);
    for (name, node) in dirs {
        if let Node::Dir(children) = node {
            let dir_path = if parent_path.is_empty() {
                name.clone()
            } else {
                format!("{parent_path}/{name}")
            };
            let is_expanded = expanded.contains(&dir_path);
            out.push(ProjectRow::Dir {
                name,
                dir_path: dir_path.clone(),
                depth,
                is_expanded,
            });
            if is_expanded {
                flatten(children, expanded, &dir_path, depth + 1, out);
            }
        }
    }
    for (name, node) in files {
        if let Node::File { full_path } = node {
            out.push(ProjectRow::File {
                name,
                full_path,
                depth,
            });
        }
    }
}

impl VcsView {
    /// Project Files 视图主入口（IDE 左侧 panel Project 模式）
    pub(super) fn render_project_files_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;

        if self.loading_project_files {
            return div()
                .px(px(2.0))
                .py(px(8.0))
                .text_sm()
                .text_color(muted_fg)
                .child("加载中...")
                .into_any_element();
        }

        // 文件路径过滤（与 Changes 视图共用同一搜索框）
        let query = self
            .files_search_input
            .read(cx)
            .value()
            .trim()
            .to_lowercase();

        // 三 key 全等命中缓存复用 rows，跳过 build_tree + flatten
        let rows_rc: Rc<Vec<ProjectRow>> = {
            let cache = self.project_rows_cache.borrow();
            let hit = cache.as_ref().filter(|e| {
                e.files_version == self.project_files_version
                    && e.expanded_version == self.project_expanded_dirs_version
                    && e.query == query
            });
            if let Some(entry) = hit {
                entry.rows.clone()
            } else {
                drop(cache);
                self.rebuild_project_rows(&query)
            }
        };

        // 空仓库 / 无匹配：缓存内 rows 也是空，给独立占位
        if rows_rc.is_empty() {
            let msg = if self.project_files.is_empty() {
                "(空仓库 / 全部文件被 .gitignore 排除)"
            } else {
                "(无匹配的文件，试着修改搜索关键词)"
            };
            return div()
                .px(px(2.0))
                .py(px(8.0))
                .text_sm()
                .text_color(muted_fg)
                .child(msg)
                .into_any_element();
        }

        // uniform_list 行级虚拟化：仅渲染屏幕可见行，万级文件也流畅
        let body = uniform_list(
            "vcs-project-files",
            rows_rc.len(),
            cx.processor({
                let rows_rc = rows_rc.clone();
                move |this, range: Range<usize>, _w, cx| {
                    range
                        .map(|i| this.render_project_row(&rows_rc[i], cx))
                        .collect::<Vec<_>>()
                }
            }),
        )
        .track_scroll(&self.project_scroll)
        .flex_1();

        v_flex()
            .size_full()
            .min_h_0()
            .child(body)
            .into_any_element()
    }

    /// 渲染单条扁平行（uniform_list closure 内调用）
    fn render_project_row(&self, row: &ProjectRow, cx: &mut Context<Self>) -> AnyElement {
        match row {
            ProjectRow::Dir {
                name,
                dir_path,
                depth,
                is_expanded,
            } => self.render_pf_dir_row(name.clone(), dir_path.clone(), *depth, *is_expanded, cx),
            ProjectRow::File {
                name,
                full_path,
                depth,
            } => self.render_pf_file_row(name.clone(), full_path.clone(), *depth, cx),
        }
    }

    /// 目录行：折叠图标 + 名字，整行可点切换展开
    fn render_pf_dir_row(
        &self,
        name: String,
        dir_path: String,
        depth: usize,
        is_expanded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted_fg = theme.muted_foreground;
        let hover_bg = theme.muted;
        let arrow = if is_expanded { "▾" } else { "▸" };
        let dir_path_for_toggle = dir_path.clone();
        let row_id = SharedString::from(format!("vcs-pf-dir-{depth}-{dir_path}"));

        h_flex()
            .id(row_id)
            .h(px(28.0))
            .flex_none()
            .w_full()
            .pl(px(4.0 + 12.0 * depth as f32))
            .gap(px(4.0))
            .items_center()
            .cursor_pointer()
            .hover(move |this| this.bg(hover_bg))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.toggle_project_dir(dir_path_for_toggle.clone(), cx);
            }))
            .child(
                div()
                    .flex_none()
                    .w(px(12.0))
                    .text_xs()
                    .text_color(muted_fg)
                    .child(arrow),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(fg)
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(name),
            )
            .into_any_element()
    }

    /// 行：状态字母 + 名字。Project 模式点文件走 select_pf_file 看内容，不走 diff
    fn render_pf_file_row(
        &self,
        name: String,
        full_path: String,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted_fg = theme.muted_foreground;
        let hover_bg = theme.muted;
        let mut accent_bg = theme.accent;
        accent_bg.a = 0.10;

        let file_status: Option<&FileStatus> = self
            .status
            .as_ref()
            .and_then(|s| s.files.iter().find(|f| f.path == full_path));
        let (letter, letter_color) = match file_status {
            Some(f) => {
                let kind = pick_display_kind(f);
                let l = code_to_letter(kind);
                (l, code_letter_color(l, muted_fg))
            }
            None => (" ", muted_fg),
        };

        // 选中态用 selected_pf_path（与 selected_file 区分，互不影响）
        let is_selected = self.selected_pf_path.as_deref() == Some(full_path.as_str());

        let path_for_open = full_path.clone();
        let row_id = SharedString::from(format!("vcs-pf-file-{depth}-{full_path}"));

        let mut row = h_flex()
            .id(row_id)
            .h(px(28.0))
            .flex_none()
            .w_full()
            .pl(px(4.0 + 12.0 * depth as f32))
            .gap(px(6.0))
            .items_center()
            .cursor_pointer()
            .hover(move |this| this.bg(hover_bg))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.select_pf_file(path_for_open.clone(), cx);
            }))
            .child(
                div()
                    .flex_none()
                    .w(px(12.0))
                    .text_xs()
                    .font_family(theme.mono_font_family.clone())
                    .text_color(letter_color)
                    .child(letter),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(fg)
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(name),
            );
        if is_selected {
            row = row.bg(accent_bg);
        }
        row.into_any_element()
    }

    /// 缓存 miss 时：filter + build_tree + flatten，结果包 Rc 并写入 cache
    ///
    /// 仅在 (files_version / expanded_version / query) 任一变化时调；命中路径直接复用。
    fn rebuild_project_rows(&self, query: &str) -> Rc<Vec<ProjectRow>> {
        // filter：搜索词非空时按 substring 过滤
        let filtered: Vec<String> = if query.is_empty() {
            self.project_files.clone()
        } else {
            self.project_files
                .iter()
                .filter(|p| p.to_lowercase().contains(query))
                .cloned()
                .collect()
        };
        // 搜索时：自动展开所有命中路径的祖先目录
        let auto_expanded: std::collections::HashSet<String> = if query.is_empty() {
            self.project_expanded_dirs.clone()
        } else {
            collect_ancestors(&filtered)
        };
        let tree = build_tree(&filtered);
        let mut rows: Vec<ProjectRow> = Vec::new();
        flatten(tree, &auto_expanded, "", 0, &mut rows);
        let rows_rc = Rc::new(rows);
        // 写回 cache（同一 render 帧内只调一次）
        *self.project_rows_cache.borrow_mut() = Some(ProjectRowsCacheEntry {
            rows: rows_rc.clone(),
            files_version: self.project_files_version,
            expanded_version: self.project_expanded_dirs_version,
            query: query.to_string(),
        });
        rows_rc
    }

    /// 切换 Project Files 目录的折叠状态
    pub(super) fn toggle_project_dir(&mut self, dir_path: String, cx: &mut Context<Self>) {
        if !self.project_expanded_dirs.remove(&dir_path) {
            self.project_expanded_dirs.insert(dir_path);
        }
        self.project_expanded_dirs_version = self.project_expanded_dirs_version.wrapping_add(1);
        cx.notify();
    }

    /// 全部展开：把仓库内所有目录路径加入 expanded set（项目大时谨慎使用）
    pub(super) fn expand_all_project_dirs(&mut self, cx: &mut Context<Self>) {
        self.project_expanded_dirs = collect_ancestors(&self.project_files);
        self.project_expanded_dirs_version = self.project_expanded_dirs_version.wrapping_add(1);
        cx.notify();
    }

    /// 全部折叠：清空 expanded set，回到默认状态（仅顶层节点可见）
    pub(super) fn collapse_all_project_dirs(&mut self, cx: &mut Context<Self>) {
        self.project_expanded_dirs.clear();
        self.project_expanded_dirs_version = self.project_expanded_dirs_version.wrapping_add(1);
        cx.notify();
    }
}

/// 把节点 map 拆成 (目录列表, 文件列表)，各自保持字母序（来自 BTreeMap）
fn split_dirs_files(map: NodeMap) -> SplitNodes {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for (name, node) in map {
        match node {
            Node::Dir(_) => dirs.push((name, node)),
            Node::File { .. } => files.push((name, node)),
        }
    }
    (dirs, files)
}

/// 文件 FileStatus → 显示状态：未暂存优先（与日常关注一致）；其次暂存；冲突最高优先
fn pick_display_kind(f: &FileStatus) -> Option<FileChangeKind> {
    if f.is_conflicted() {
        return Some(FileChangeKind::Conflicted);
    }
    f.unstaged.or(f.staged)
}

/// 搜索时收集所有命中路径的祖先目录（让用户能看到匹配文件）
///
/// 例：`["a/b/c.rs"]` → `{"a", "a/b"}`
fn collect_ancestors(paths: &[String]) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    for p in paths {
        let parts: Vec<&str> = p.split('/').collect();
        for i in 1..parts.len() {
            set.insert(parts[..i].join("/"));
        }
    }
    set
}
