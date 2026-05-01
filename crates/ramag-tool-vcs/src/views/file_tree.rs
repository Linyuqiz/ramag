//! 文件路径树共享 helper：把扁平 `Vec<FileStatus>` 构建成嵌套目录树并扁平化为 Row 列表
//!
//! Commit Detail 与 Changes 共用此 helper，保证两处文件列表都按目录组织 + 中间空目录压缩
//! （IDEA 风格 compact middle packages，让深层路径只占一行）

use std::collections::{BTreeMap, HashSet};

use ramag_domain::entities::FileStatus;

/// 树节点：目录 / 文件
pub(super) enum Node {
    Dir(BTreeMap<String, Node>),
    File { idx: usize },
}

/// 扁平化后的一行（uniform_list 数据单元，强制等高）
#[derive(Clone)]
pub(super) enum Row {
    Dir {
        display_name: String,
        dir_path: String,
        depth: usize,
        is_collapsed: bool,
        file_count: usize,
    },
    File {
        idx: usize,
        depth: usize,
    },
}

/// 把扁平 file 列表构建成嵌套目录树（按 / 分割路径）
pub(super) fn build_tree(files: &[FileStatus]) -> BTreeMap<String, Node> {
    let mut root: BTreeMap<String, Node> = BTreeMap::new();
    for (idx, f) in files.iter().enumerate() {
        let parts: Vec<&str> = f.path.split('/').collect();
        if parts.is_empty() {
            continue;
        }
        insert_path(&mut root, &parts, idx);
    }
    root
}

fn insert_path(map: &mut BTreeMap<String, Node>, parts: &[&str], idx: usize) {
    if parts.is_empty() {
        return;
    }
    if parts.len() == 1 {
        map.insert(parts[0].to_string(), Node::File { idx });
        return;
    }
    let dir = parts[0];
    let entry = map
        .entry(dir.to_string())
        .or_insert_with(|| Node::Dir(BTreeMap::new()));
    if let Node::Dir(children) = entry {
        insert_path(children, &parts[1..], idx);
    }
}

/// 扁平化树：dir 自动压缩单链中间目录（IDEA compact middle packages）
pub(super) fn flatten(
    map: &BTreeMap<String, Node>,
    depth: usize,
    prefix: &str,
    collapsed: &HashSet<String>,
    out: &mut Vec<Row>,
) {
    let mut dirs: Vec<(String, &BTreeMap<String, Node>)> = Vec::new();
    let mut files: Vec<(String, usize)> = Vec::new();
    for (name, node) in map {
        match node {
            Node::Dir(children) => dirs.push((name.clone(), children)),
            Node::File { idx } => files.push((name.clone(), *idx)),
        }
    }
    for (name, children) in dirs {
        let mut display = name.clone();
        let mut full = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let mut cur = children;
        while cur.len() == 1 {
            let Some((only_name, Node::Dir(grandchildren))) = cur.iter().next() else {
                break;
            };
            display = format!("{display}/{only_name}");
            full = format!("{full}/{only_name}");
            cur = grandchildren;
        }
        let is_collapsed = collapsed.contains(&full);
        let file_count = count_files(cur);
        out.push(Row::Dir {
            display_name: display,
            dir_path: full.clone(),
            depth,
            is_collapsed,
            file_count,
        });
        if !is_collapsed {
            flatten(cur, depth + 1, &full, collapsed, out);
        }
    }
    for (_name, idx) in files {
        out.push(Row::File { idx, depth });
    }
}

fn count_files(map: &BTreeMap<String, Node>) -> usize {
    let mut total = 0;
    for node in map.values() {
        match node {
            Node::Dir(children) => total += count_files(children),
            Node::File { .. } => total += 1,
        }
    }
    total
}
