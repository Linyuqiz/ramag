//! VCS（Git）UI 层。底层 ramag-infra-git；尚未抽 VcsService，view 直调 driver

pub mod actions;
pub mod views;
mod watcher;

pub use actions::{
    CommitNow, FocusCommitMessage, PullNow, PushNow, RefreshWorkspace, ToggleHistoryPane,
};
pub use views::vcs_view::VcsView;

use std::sync::Arc;

use gpui::{App, AppContext as _, Entity, Window};
use ramag_domain::traits::{GitDriver, Storage, Tool, ToolMeta};

/// storage 用于 recent_repos 持久化
pub fn create_vcs_view(
    driver: Arc<dyn GitDriver>,
    storage: Arc<dyn Storage>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<VcsView> {
    cx.new(|cx_inner| VcsView::new(driver, storage, window, cx_inner))
}

pub struct VcsTool {
    meta: ToolMeta,
}

impl VcsTool {
    pub const ID: &'static str = "vcs";

    pub fn new() -> Self {
        Self {
            meta: ToolMeta::new(
                Self::ID,
                "版本管理",
                "Git 可视化客户端：仓库管理 / commit / 分支 / 推拉合并",
            )
            .with_icon("git_branch"),
        }
    }
}

impl Default for VcsTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for VcsTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
}
