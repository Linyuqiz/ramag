//! 剪贴板工具：历史卡片流 + 搜索/筛选 + 设置。
//! 采集循环在 ramag-bin 的 App 级 spawn 中运行（独立于本视图生死）

pub mod actions;
pub mod views;

pub use actions::{
    CopySelectedClip, DeleteSelectedClip, FocusClipSearch, SelectNextClip, SelectPrevClip,
};
pub use views::{ClipboardDrawer, ClipboardView};

use std::sync::Arc;

use gpui::{App, AppContext as _, Entity, Window};
use ramag_app::ClipboardService;
use ramag_domain::traits::{Tool, ToolMeta};

/// 创建剪贴板主视图（由 ramag-bin 注入 service 后注册进 Shell）
pub fn create_clipboard_view(
    service: Arc<ClipboardService>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ClipboardView> {
    cx.new(|cx| ClipboardView::new(service, window, cx))
}

/// 创建底部悬浮抽屉视图（由 ramag-bin 在 PopUp 窗口内装载）。
/// target_bundle 为唤起时的前台应用 bundle id，用于粘贴后激活回去
pub fn create_clipboard_drawer(
    service: Arc<ClipboardService>,
    target_bundle: Option<String>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ClipboardDrawer> {
    cx.new(|cx| ClipboardDrawer::new(service, target_bundle, window, cx))
}

pub struct ClipboardTool {
    meta: ToolMeta,
}

impl ClipboardTool {
    pub const ID: &'static str = "clipboard";

    pub fn new() -> Self {
        Self {
            meta: ToolMeta::new(
                Self::ID,
                "剪贴板",
                "剪贴历史：搜索 / 筛选 / 快速粘贴，全本地加密",
            )
            .with_icon("clipboard"),
        }
    }
}

impl Default for ClipboardTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ClipboardTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
}
