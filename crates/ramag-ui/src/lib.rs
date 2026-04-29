//! Ramag 共享 UI 模块
//!
//! 提供：
//! - **Shell**：主壳（左侧 Tool 列表 + 右侧 Tool 视图区）
//! - 后续会加入主题/通用组件

pub mod actions;
pub mod activity_bar;
pub mod assets;
pub mod home_view;
pub mod icons;
pub mod shell;
pub mod theme;

pub use actions::CloseTab;
pub use assets::RamagAssets;

pub use activity_bar::{ActivityBar, NavEvent, NavTarget};
pub use home_view::{HomeEvent, HomeView};
pub use shell::Shell;
pub use theme::{
    Mode, StorageGlobal, apply_theme, current_mode, init_theme, on_system_appearance_changed,
    toggle_theme,
};
