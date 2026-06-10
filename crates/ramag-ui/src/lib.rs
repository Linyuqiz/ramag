//! 共享 UI：Shell（左 ActivityBar + 右 Tool 视图）+ 主题 + 通用组件

pub mod actions;
pub mod activity_bar;
pub mod assets;
pub mod confirm_dialog;
pub mod home_view;
pub mod icons;
pub mod shell;
pub mod theme;

pub use actions::CloseTab;
pub use assets::RamagAssets;
pub use confirm_dialog::open_confirm;

pub use activity_bar::{ActivityBar, NavEvent, NavTarget};
pub use home_view::{HomeEvent, HomeView};
pub use shell::Shell;
pub use theme::{
    Mode, StorageGlobal, apply_theme, current_mode, init_theme, on_system_appearance_changed,
};
