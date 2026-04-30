//! Ramag 自有图标的便捷工厂
//!
//! 这些 svg 由 [`crate::assets::RamagAssets`] 在运行时通过 `Icon::path()` 加载，
//! 不依赖 `gpui_component::IconName`（IconName 由上游宏从上游 svg 目录扫描生成，
//! 自然不包含 ramag 自己的图标）。
//!
//! 使用方式：`Button::new(...).icon(icons::database())`

use gpui_component::Icon;

#[inline]
pub fn home() -> Icon {
    Icon::default().path("icons/home.svg")
}

#[inline]
pub fn database() -> Icon {
    Icon::default().path("icons/database.svg")
}

#[inline]
pub fn git_branch() -> Icon {
    Icon::default().path("icons/git-branch.svg")
}

#[inline]
pub fn refresh_cw() -> Icon {
    Icon::default().path("icons/refresh-cw.svg")
}

#[inline]
pub fn wand_sparkles() -> Icon {
    Icon::default().path("icons/wand-sparkles.svg")
}

#[inline]
pub fn gauge() -> Icon {
    Icon::default().path("icons/gauge.svg")
}

#[inline]
pub fn download() -> Icon {
    Icon::default().path("icons/download.svg")
}

#[inline]
pub fn pencil() -> Icon {
    Icon::default().path("icons/pencil.svg")
}

#[inline]
pub fn trash() -> Icon {
    Icon::default().path("icons/trash-2.svg")
}
