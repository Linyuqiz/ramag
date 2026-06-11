//! ramag 自有 svg 的图标工厂。runtime 经 RamagAssets 加载，绕开上游 IconName 编译期扫描

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

#[inline]
pub fn git_commit() -> Icon {
    Icon::default().path("icons/git-commit.svg")
}

#[inline]
pub fn git_merge() -> Icon {
    Icon::default().path("icons/git-merge.svg")
}

#[inline]
pub fn circle_dot() -> Icon {
    Icon::default().path("icons/circle-dot.svg")
}

#[inline]
pub fn scroll_text() -> Icon {
    Icon::default().path("icons/scroll-text.svg")
}

#[inline]
pub fn columns_2() -> Icon {
    Icon::default().path("icons/columns-2.svg")
}

#[inline]
pub fn list_filter() -> Icon {
    Icon::default().path("icons/list-filter.svg")
}

#[inline]
pub fn clipboard() -> Icon {
    Icon::default().path("icons/clipboard.svg")
}

#[inline]
pub fn pin() -> Icon {
    Icon::default().path("icons/pin.svg")
}

#[inline]
pub fn pin_off() -> Icon {
    Icon::default().path("icons/pin-off.svg")
}

#[inline]
pub fn settings() -> Icon {
    Icon::default().path("icons/settings.svg")
}

#[inline]
pub fn copy() -> Icon {
    Icon::default().path("icons/copy.svg")
}

#[inline]
pub fn checker() -> Icon {
    Icon::default().path("icons/checker.svg")
}
