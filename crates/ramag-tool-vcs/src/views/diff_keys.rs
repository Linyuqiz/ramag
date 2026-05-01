//! Diff 虚拟化扁平 key：把 FileDiff hunks 转成 uniform_list 索引序列
//!
//! 抽出独立模块让 diff_panel.rs 保持 < 600 行。两种 key：unified（每行单独）
//! 和 split（左右配对，删除/新增对齐）。

use std::collections::HashSet;

use ramag_domain::entities::{DiffLineKind, FileDiff};

/// Unified 模式扁平 key：hunk header 或单行
#[derive(Clone, Copy)]
pub(super) enum UnifiedKey {
    Header { hunk_idx: usize },
    Line { hunk_idx: usize, line_idx: usize },
}

/// Split 模式扁平 key：hunk header / 左右配对行 / Spacer 间隔分隔
///
/// 全部用 Pair 渲染（删除行 left=Some/right=None；新增行 left=None/right=Some），
/// 保持 split 视觉对称：删/增不跨栏，对侧空白对齐。
/// Spacer：连续 ≥SPLIT_SPACER_THRESHOLD 行 Context 时压缩中间，让逻辑上不相干的两段变更视觉拆开
#[derive(Clone, Copy)]
pub(super) enum SplitKey {
    Header {
        hunk_idx: usize,
    },
    Pair {
        hunk_idx: usize,
        left: Option<usize>,
        right: Option<usize>,
    },
    Spacer {
        /// 所属 hunk 索引（点击展开时与 run_start 一起作为 expanded_diff_spacers 的 key）
        hunk_idx: usize,
        /// 该 Context 段的首行 line_idx（同 hunk 内 spacer 唯一标识）
        run_start: usize,
        /// 被折叠的行数（首尾 KEEP 行不算）
        skipped: usize,
    },
}

/// 连续 Context 行数阈值：超过此数视觉拆开两段变更（保留首尾各 SPLIT_SPACER_KEEP 行）
const SPLIT_SPACER_THRESHOLD: usize = 6;
const SPLIT_SPACER_KEEP: usize = 2;

/// 把 FileDiff 扁平化成 UnifiedKey 序列（hunk header + 每行）
///
/// `changes_only=true` 时跳过 Context 行；hunk 内全是 context（无变更）也不渲染该 hunk header
pub(super) fn build_unified_keys(diff: &FileDiff, changes_only: bool) -> Vec<UnifiedKey> {
    let mut out = Vec::new();
    for (h_idx, h) in diff.hunks.iter().enumerate() {
        let has_change = !changes_only
            || h.lines
                .iter()
                .any(|l| !matches!(l.kind, DiffLineKind::Context));
        if !has_change {
            continue;
        }
        out.push(UnifiedKey::Header { hunk_idx: h_idx });
        for (l_idx, line) in h.lines.iter().enumerate() {
            if changes_only && matches!(line.kind, DiffLineKind::Context) {
                continue;
            }
            out.push(UnifiedKey::Line {
                hunk_idx: h_idx,
                line_idx: l_idx,
            });
        }
    }
    out
}

/// 把 FileDiff 扁平化成 SplitKey 序列（删除/新增按出现顺序左右配对）
///
/// `changes_only=true` 时跳过 Context 行；只保留删除/新增配对。
/// 大块连续 Context（≥SPLIT_SPACER_THRESHOLD）时压缩中间，插入 Spacer 让视觉拆开两段变更。
/// `expanded_spacers` 包含 (hunk_idx, run_start) 的 spacer 不再压缩，全部展开（用户点击触发）
pub(super) fn build_split_keys(
    diff: &FileDiff,
    changes_only: bool,
    expanded_spacers: &HashSet<(usize, usize)>,
) -> Vec<SplitKey> {
    let mut out = Vec::new();
    for (h_idx, h) in diff.hunks.iter().enumerate() {
        let has_change = !changes_only
            || h.lines
                .iter()
                .any(|l| !matches!(l.kind, DiffLineKind::Context));
        if !has_change {
            continue;
        }
        out.push(SplitKey::Header { hunk_idx: h_idx });
        let mut pending_left: Vec<usize> = Vec::new();
        let mut pending_right: Vec<usize> = Vec::new();
        // 当前 Context 段（连续 Context 行的索引集合）
        let mut ctx_run: Vec<usize> = Vec::new();
        let flush_ctx = |run: &mut Vec<usize>, out: &mut Vec<SplitKey>| {
            if run.is_empty() {
                return;
            }
            let run_start = run[0];
            let user_expanded = expanded_spacers.contains(&(h_idx, run_start));
            if !changes_only && run.len() >= SPLIT_SPACER_THRESHOLD && !user_expanded {
                // 保留前 KEEP 行 + Spacer + 后 KEEP 行
                let n = run.len();
                for &i in run.iter().take(SPLIT_SPACER_KEEP) {
                    out.push(SplitKey::Pair {
                        hunk_idx: h_idx,
                        left: Some(i),
                        right: Some(i),
                    });
                }
                out.push(SplitKey::Spacer {
                    hunk_idx: h_idx,
                    run_start,
                    skipped: n - SPLIT_SPACER_KEEP * 2,
                });
                for &i in run.iter().skip(n - SPLIT_SPACER_KEEP) {
                    out.push(SplitKey::Pair {
                        hunk_idx: h_idx,
                        left: Some(i),
                        right: Some(i),
                    });
                }
            } else if !changes_only {
                // 短段（< 阈值）OR 用户已点击展开 → 全部 Context 行铺开
                for &i in run.iter() {
                    out.push(SplitKey::Pair {
                        hunk_idx: h_idx,
                        left: Some(i),
                        right: Some(i),
                    });
                }
            }
            run.clear();
        };
        for (i, line) in h.lines.iter().enumerate() {
            match line.kind {
                DiffLineKind::Delete => {
                    flush_ctx(&mut ctx_run, &mut out);
                    pending_left.push(i);
                }
                DiffLineKind::Add => {
                    flush_ctx(&mut ctx_run, &mut out);
                    pending_right.push(i);
                }
                DiffLineKind::Context => {
                    flush_pairs(h_idx, &mut pending_left, &mut pending_right, &mut out);
                    ctx_run.push(i);
                }
            }
        }
        flush_pairs(h_idx, &mut pending_left, &mut pending_right, &mut out);
        flush_ctx(&mut ctx_run, &mut out);
    }
    out
}

fn flush_pairs(
    hunk_idx: usize,
    left: &mut Vec<usize>,
    right: &mut Vec<usize>,
    out: &mut Vec<SplitKey>,
) {
    // 全部配对（左旧右新，对侧空时 None）—— 保持 split 视觉对称：
    // 删除行只在左栏，新增行只在右栏，对侧空白对齐，不跨栏
    let n = left.len().max(right.len());
    for i in 0..n {
        out.push(SplitKey::Pair {
            hunk_idx,
            left: left.get(i).copied(),
            right: right.get(i).copied(),
        });
    }
    left.clear();
    right.clear();
}
