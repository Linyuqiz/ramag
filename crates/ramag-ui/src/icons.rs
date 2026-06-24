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
pub fn settings() -> Icon {
    Icon::default().path("icons/settings.svg")
}

#[inline]
pub fn copy() -> Icon {
    Icon::default().path("icons/copy.svg")
}

/// 数据库官方品牌彩色 logo 的内嵌资源路径。
///
/// 与上面单色 `Icon` 工厂不同：品牌 logo 是多色 SVG，必须经 `gpui::img()` 光栅化渲染以
/// 保留原色；不能走 `Icon`（会被 `text_color` 压成单色）。未知 driver 返回 `None`，调用方
/// 回退到通用 `database()` 图标。driver_id 取值与 dbclient 的 `DRIVERS` 常量一致。
#[inline]
pub fn db_brand_icon(driver_id: &str) -> Option<&'static str> {
    Some(match driver_id {
        "mysql" => "icons/db-mysql.svg",
        "postgres" => "icons/db-postgresql.svg",
        "redis" => "icons/db-redis.svg",
        "mongodb" => "icons/db-mongodb.svg",
        _ => return None,
    })
}
