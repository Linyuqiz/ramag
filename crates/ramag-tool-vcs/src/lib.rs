//! Ramag VCS（Git）可视化工具
//!
//! Phase A 骨架：仅注册 Tool 元数据，提供最小 view（仓库选择 + 状态预览）；
//! Phase B+ 逐步补：commit / diff / branch / push / pull / stash / log 等。
//!
//! 与 dbclient 同分层：
//! - 本 crate（tool-vcs）：UI 层
//! - `ramag-infra-git`：gix 实现 GitDriver trait
//! - `ramag-app::VcsService`：Use case 聚合（暂未抽出，先在 view 内直调 driver）
//! - `ramag-domain`：实体 + GitDriver trait

pub mod views;

pub use views::vcs_view::{VcsView, create_vcs_view};

use ramag_domain::traits::{Tool, ToolMeta};

/// 版本管理工具（Git 客户端）
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
