//! Commit graph：lane 分配算法 + lane 着色 + 左侧 gutter 渲染
//!
//! 抽自 helpers.rs（让其不超 600 行）。算法见 [`build_commit_lanes`] 注释。

use gpui::{AnyElement, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{Icon, Sizable as _, h_flex, v_flex};
use ramag_domain::entities::{Commit, CommitId};

/// 单条 commit 在 history 视图中的图谱位置
#[derive(Debug, Clone)]
pub(super) struct CommitGraphRow {
    /// 该 commit 占的 lane 索引（0 = 最左）
    pub(super) lane: usize,
    /// 当前行总共有多少条活跃 lane（决定 gutter 宽度）
    pub(super) total_lanes: usize,
    /// 是否 merge commit（多 parent，dot 替换为 git-merge 图标）
    pub(super) is_merge: bool,
}

/// 单条 lane 宽度（px）：要小于 commit dot 直径，让线刚好被 dot 覆盖
const LANE_WIDTH: f32 = 14.0;

/// 把按时间倒序的 commit 列表转成 lane 分配序列
///
/// 算法（线性 + 偶尔分叉的实用近似）：
/// 1. 维护 `active`：每条 lane 当前在等的 commit（FIFO 复用空 slot）
/// 2. 当前 commit 的 lane = `active` 中等它的位置；找不到就用空 slot 或新增
/// 3. 处理后把 `active[lane]` 替换为 first parent；若 first parent 已在别的 lane 等
///    （即 lane 合并到主线），则本 lane 终结
/// 4. 其余 parent（merge commit 时）入新 lane
pub(super) fn build_commit_lanes(commits: &[Commit]) -> Vec<CommitGraphRow> {
    let mut active: Vec<Option<CommitId>> = Vec::new();
    let mut rows: Vec<CommitGraphRow> = Vec::with_capacity(commits.len());

    for c in commits {
        let mut lane_idx = active.iter().position(|x| x.as_ref() == Some(&c.id));
        if lane_idx.is_none() {
            lane_idx = match active.iter().position(Option::is_none) {
                Some(empty) => Some(empty),
                None => {
                    active.push(None);
                    Some(active.len() - 1)
                }
            };
        }
        let lane = lane_idx.unwrap_or(0);
        let is_merge = c.parents.len() > 1;

        if let Some(p0) = c.parents.first() {
            let p0_in_other = active
                .iter()
                .enumerate()
                .any(|(i, x)| i != lane && x.as_ref() == Some(p0));
            active[lane] = if p0_in_other { None } else { Some(p0.clone()) };
        } else {
            active[lane] = None;
        }

        for p in c.parents.iter().skip(1) {
            let already = active.iter().any(|x| x.as_ref() == Some(p));
            if already {
                continue;
            }
            match active.iter().position(Option::is_none) {
                Some(empty) => active[empty] = Some(p.clone()),
                None => active.push(Some(p.clone())),
            }
        }

        let mut total = active.len();
        while total > 0 && active[total - 1].is_none() {
            total -= 1;
        }
        let total_lanes = total.max(lane + 1);

        rows.push(CommitGraphRow {
            lane,
            total_lanes,
            is_merge,
        });
    }
    rows
}

/// 给 lane 分配高对比度颜色（基于黄金角分布，相邻 lane 不会同色）
pub(super) fn lane_color(lane: usize) -> gpui::Hsla {
    // 黄金角 137.508°：连续 hash 后相邻值 hue 差最大
    let hue = (lane as f32 * 137.508) % 360.0;
    gpui::hsla(hue / 360.0, 0.55, 0.55, 1.0)
}

/// 渲染左侧 lane gutter：N 条彩色竖线 + 本 commit 所在 lane 的 dot
pub(super) fn render_lane_gutter(graph: &CommitGraphRow) -> AnyElement {
    let total = graph.total_lanes.max(1);
    let mut row = h_flex().flex_none().items_stretch();
    for i in 0..total {
        let mut color = lane_color(i);
        let mut bg_line = color;
        bg_line.a = 0.45;
        let lane_div = if i == graph.lane {
            let dot_icon: AnyElement = if graph.is_merge {
                Icon::new(ramag_ui::icons::git_merge())
                    .small()
                    .text_color(color)
                    .into_any_element()
            } else {
                color.a = 1.0;
                Icon::new(ramag_ui::icons::circle_dot())
                    .small()
                    .text_color(color)
                    .into_any_element()
            };
            v_flex()
                .flex_none()
                .w(px(LANE_WIDTH))
                .items_center()
                .child(div().w(px(2.0)).h(px(8.0)).bg(bg_line))
                .child(dot_icon)
                .child(div().w(px(2.0)).flex_1().bg(bg_line))
        } else {
            v_flex()
                .flex_none()
                .w(px(LANE_WIDTH))
                .items_center()
                .child(div().w(px(2.0)).h_full().bg(bg_line))
        };
        row = row.child(lane_div);
    }
    row.into_any_element()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use ramag_domain::entities::{CommitId, Signature};

    fn mk(id: &str, parents: &[&str]) -> Commit {
        let sig = Signature {
            name: "Author".into(),
            email: "a@e.com".into(),
            timestamp: Utc.timestamp_opt(0, 0).unwrap(),
        };
        Commit {
            id: CommitId(id.into()),
            parents: parents.iter().map(|p| CommitId((*p).into())).collect(),
            author: sig.clone(),
            committer: sig,
            subject: format!("commit {id}"),
            body: String::new(),
            refs: Vec::new(),
        }
    }

    #[test]
    fn linear_history_keeps_one_lane() {
        let commits = vec![mk("c", &["b"]), mk("b", &["a"]), mk("a", &[])];
        let rows = build_commit_lanes(&commits);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.lane == 0));
        assert!(rows.iter().all(|r| r.total_lanes == 1));
    }

    #[test]
    fn merge_commit_uses_two_lanes_then_collapses() {
        let commits = vec![
            mk("m", &["p1", "p2"]),
            mk("p1", &["r"]),
            mk("p2", &["r"]),
            mk("r", &[]),
        ];
        let rows = build_commit_lanes(&commits);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].lane, 0);
        assert!(rows[0].is_merge);
        assert_eq!(rows[0].total_lanes, 2);
        assert_eq!(rows[1].lane, 0);
        assert_eq!(rows[2].lane, 1);
        assert_eq!(rows[3].lane, 0);
        assert_eq!(rows[3].total_lanes, 1);
    }

    #[test]
    fn lane_color_is_deterministic_per_lane() {
        let c0 = lane_color(0);
        let c1 = lane_color(1);
        assert!((c0.h - c1.h).abs() > 0.001);
        assert!((c0.h - lane_color(0).h).abs() < 1e-6);
    }
}
