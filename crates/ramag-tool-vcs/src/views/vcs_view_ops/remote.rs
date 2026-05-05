//! VcsView Remote 异步操作：fetch / pull / push（含 force-with-lease）

use gpui::Context;
use ramag_domain::entities::BranchKind;
use tracing::{error, info};

use super::super::helpers::RemoteOp;
use super::super::vcs_view::VcsView;

impl VcsView {
    /// 远程同步：fetch / pull / push
    ///
    /// remote 解析策略：
    /// - 当前分支有 upstream（如 "origin/main"）→ 拆出 remote 名 + 远端分支名
    /// - 当前分支无 upstream → push 自动加 -u 设置；pull 报错引导用户先 push
    /// - fetch 总是 `git fetch --all --prune`，多 remote 一并拉
    pub(in crate::views) fn run_remote_op(&mut self, op: RemoteOp, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let Some(local_branch) = self.status.as_ref().and_then(|s| s.head_branch.clone()) else {
            self.error = Some("当前为 detached HEAD，无法 push/pull".into());
            cx.notify();
            return;
        };
        // 从 local_branches 找当前 head 的 upstream（"origin/main"）
        let upstream = self
            .local_branches
            .iter()
            .find(|b| b.is_head)
            .and_then(|b| b.upstream.clone());
        let (remote_name, remote_branch) = match upstream.as_deref().and_then(|u| u.split_once('/'))
        {
            Some((r, b)) => (r.to_string(), b.to_string()),
            None => ("origin".to_string(), local_branch.clone()),
        };
        let need_set_upstream = upstream.is_none();
        // PushForce → 走 --force-with-lease；其他 op 忽略
        let this_force_lease = matches!(op, RemoteOp::PushForce);
        // pull 模式下若没有 upstream 直接报错引导（避免提示「fatal: no tracking info」）
        if matches!(op, RemoteOp::Pull) && need_set_upstream {
            self.error =
                Some("当前分支没有上游分支：先点 Push（会自动设置 upstream）再 Pull".into());
            cx.notify();
            return;
        }
        let driver = self.driver.clone();
        self.busy = true;
        self.error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = match op {
                // 空 remote 让 driver 拉所有 remote
                RemoteOp::Fetch => driver.fetch(&repo, "").await,
                RemoteOp::Pull => {
                    driver
                        .pull(&repo, &remote_name, &remote_branch, false)
                        .await
                }
                RemoteOp::Push | RemoteOp::PushForce => {
                    driver
                        .push(
                            &repo,
                            &remote_name,
                            &local_branch,
                            need_set_upstream,
                            this_force_lease,
                        )
                        .await
                }
            };
            // 不论成功失败都刷新一次 status（pull 后 ahead/behind 必变）
            let new_status = driver.status(&repo).await.ok();
            let local = driver.list_branches(&repo, BranchKind::Local).await.ok();
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                if let Err(e) = result {
                    error!(error = %e, ?op, "vcs: remote op failed");
                    this.error = Some(format!("{op:?} 失败：{e}"));
                } else {
                    info!(?op, "vcs: remote op done");
                }
                if let Some(s) = new_status {
                    this.status = Some(s);
                }
                if let Some(b) = local {
                    this.local_branches = b;
                }
                cx.notify();
            });
        })
        .detach();
    }
}
