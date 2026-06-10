//! 扁平 key 按 `:` 建 Trie 多层命名空间树

use std::collections::HashSet;

use ramag_domain::entities::{KeyMeta, RedisType};

use super::NAMESPACE_SEP;

/// 树节点：可同时是命名空间（有子节点）和叶子（对应实际 key）
#[derive(Debug, Clone)]
pub(super) struct TreeNode {
    /// 当前层显示标签（路径中的一段）
    pub(super) label: String,
    /// 完整路径（叶子时是完整 key 名；中间节点是路径前缀）
    pub(super) full_path: String,
    /// 子节点（按 label 排序：命名空间在前，叶子在后；同类按字母升序）
    pub(super) children: Vec<TreeNode>,
    /// 该节点本身是否对应实际 key（SCAN 不查类型，bare key 的 leaf_type 为 None，
    /// 不能用 leaf_type.is_some() 判定叶子）
    pub(super) is_key: bool,
    /// key 的类型（None=未查询；仅用于类型徽标显示）
    pub(super) leaf_type: Option<RedisType>,
}

impl TreeNode {
    pub(super) fn is_namespace(&self) -> bool {
        !self.children.is_empty()
    }
}

/// 渲染层用的扁平行（拥有数据，避免与 cx.listener 借用冲突）
#[derive(Debug, Clone)]
pub(super) struct VisibleRow {
    pub(super) depth: usize,
    pub(super) label: String,
    pub(super) full_path: String,
    pub(super) is_key: bool,
    pub(super) leaf_type: Option<RedisType>,
    pub(super) is_namespace: bool,
    pub(super) is_expanded: bool,
}

pub(super) fn build_tree(keys: &[KeyMeta]) -> Vec<TreeNode> {
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
            nodes[p].is_key = true;
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
            is_key: false,
            leaf_type: None,
        };
        if is_last {
            new_node.full_path = full_key;
            new_node.is_key = true;
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
pub(super) fn has_match_descendant(node: &TreeNode, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    if node.is_key && node.full_path.to_lowercase().contains(query) {
        return true;
    }
    for c in &node.children {
        if c.full_path.to_lowercase().contains(query) || has_match_descendant(c, query) {
            return true;
        }
    }
    false
}

pub(super) fn collect_namespace_paths(node: &TreeNode, out: &mut HashSet<String>) {
    if node.is_namespace() {
        out.insert(node.full_path.clone());
        for c in &node.children {
            collect_namespace_paths(c, out);
        }
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
        assert!(tree.iter().all(|n| n.is_namespace()));
        let labels: Vec<_> = tree.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(labels, vec!["session", "user"]);
    }

    #[test]
    fn leaf_and_namespace_coexist() {
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

    /// SCAN 装载的 bare key（key_type=None）必须仍被识别为叶子：
    /// 右键删除菜单与搜索匹配都依赖 is_key，而非 leaf_type
    #[test]
    #[allow(clippy::expect_used)]
    fn bare_key_is_still_leaf() {
        let keys = vec![KeyMeta::bare("111"), KeyMeta::bare("user:1")];
        let tree = build_tree(&keys);
        let root_111 = tree.iter().find(|n| n.label == "111").expect("有 111 节点");
        assert!(root_111.is_key, "bare key 应被标记为叶子");
        assert!(
            root_111.leaf_type.is_none(),
            "未查询类型时 leaf_type 保持 None"
        );
        assert!(
            has_match_descendant(root_111, "111"),
            "搜索应能命中 bare key"
        );

        let user = tree
            .iter()
            .find(|n| n.label == "user")
            .expect("有 user 节点");
        assert!(!user.is_key, "纯命名空间不是叶子");
        assert!(user.children[0].is_key);
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
        assert!(paths.contains("a"));
        assert!(paths.contains("a:b"));
        assert!(!paths.contains("a:b:c"));
        assert!(!paths.contains("a:d"));
    }
}
