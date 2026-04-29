//! Ramag 主题：VSCode 风暗色 / 浅色色板
//!
//! 启动时调用 [`apply_theme(Mode::Dark, cx)`]，运行中可调 [`toggle_theme`] 切换。

use std::sync::Arc;

use gpui::{App, Global, Hsla, WindowAppearance, hsla};
use gpui_component::{Theme, ThemeMode};
use ramag_domain::traits::Storage;

/// gpui Global 容器：让 UI 层在切换主题时能访问 Storage 做持久化
pub struct StorageGlobal(pub Arc<dyn Storage>);
impl Global for StorageGlobal {}

/// 从 cx 取出 StorageGlobal（main 可能没注入，返回 None 时不持久化）
pub fn storage_from_cx(cx: &App) -> Option<Arc<dyn Storage>> {
    cx.try_global::<StorageGlobal>().map(|g| g.0.clone())
}

/// 当前主题是否处于"跟随系统"状态
///
/// - true：preference 未显式设过 dark/light；系统外观变化时自动同步
/// - false：用户显式选过 Dark/Light；忽略系统外观变化
pub struct FollowSystem(pub bool);
impl Global for FollowSystem {}

pub fn is_following_system(cx: &App) -> bool {
    cx.try_global::<FollowSystem>()
        .map(|g| g.0)
        .unwrap_or(false)
}

pub fn set_following_system(cx: &mut App, follow: bool) {
    cx.set_global(FollowSystem(follow));
}

/// Ramag 自定义主题模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

impl Mode {
    pub fn toggled(self) -> Self {
        match self {
            Mode::Dark => Mode::Light,
            Mode::Light => Mode::Dark,
        }
    }
}

/// macOS WindowAppearance → ramag Mode
pub fn mode_from_appearance(appearance: WindowAppearance) -> Mode {
    match appearance {
        WindowAppearance::Dark | WindowAppearance::VibrantDark => Mode::Dark,
        _ => Mode::Light,
    }
}

/// 启动时根据 preference + 系统外观初始化主题
///
/// - preference == None / "system" → 跟随系统：用 `appearance` 决定 Mode，follow_system=true
/// - preference == "dark" / "light" → 用户偏好：直接用该值，follow_system=false
pub fn init_theme(preference: Option<&str>, appearance: WindowAppearance, cx: &mut App) {
    let (mode, follow) = match preference {
        Some("dark") => (Mode::Dark, false),
        Some("light") => (Mode::Light, false),
        _ => (mode_from_appearance(appearance), true),
    };
    apply_theme(mode, cx);
    set_following_system(cx, follow);
}

/// 系统外观变化的回调入口（由 Shell.observe_window_appearance 触发）
///
/// 仅当 follow_system 时才实际重应用主题；用户已显式选过的不动
pub fn on_system_appearance_changed(appearance: WindowAppearance, cx: &mut App) {
    if !is_following_system(cx) {
        return;
    }
    let mode = mode_from_appearance(appearance);
    if current_mode(cx) != mode {
        apply_theme(mode, cx);
        cx.refresh_windows();
    }
}

/// 应用主题
pub fn apply_theme(mode: Mode, cx: &mut App) {
    let base = match mode {
        Mode::Dark => ThemeMode::Dark,
        Mode::Light => ThemeMode::Light,
    };
    Theme::change(base, None, cx);
    let theme = Theme::global_mut(cx);
    match mode {
        Mode::Dark => apply_dark_palette(theme),
        Mode::Light => apply_light_palette(theme),
    }
}

/// 切换 dark↔light 并立即生效
pub fn toggle_theme(cx: &mut App) -> Mode {
    let current = current_mode(cx);
    let next = current.toggled();
    apply_theme(next, cx);
    cx.refresh_windows();
    next
}

/// 通过 Theme::global 推断当前模式（依赖 Theme.mode 字段）
pub fn current_mode(cx: &App) -> Mode {
    let theme = Theme::global(cx);
    if matches!(theme.mode, ThemeMode::Light) {
        Mode::Light
    } else {
        Mode::Dark
    }
}

/// VSCode Dark+ 配色
fn apply_dark_palette(theme: &mut Theme) {
    // 主色：VSCode 蓝（#007ACC 风）
    let accent = hsl(207.0, 100.0, 42.0);
    let accent_hover = hsl(207.0, 100.0, 50.0);
    let accent_active = hsl(207.0, 100.0, 36.0);

    theme.accent = accent;
    theme.accent_foreground = hsl(0.0, 0.0, 100.0);
    theme.primary = accent;
    theme.primary_hover = accent_hover;
    theme.primary_active = accent_active;
    theme.primary_foreground = hsl(0.0, 0.0, 100.0);

    theme.link = accent_hover;
    theme.link_hover = hsl(207.0, 100.0, 60.0);
    theme.link_active = accent_active;

    // 背景三段灰
    theme.background = hsl(0.0, 0.0, 12.0); // #1E1E1E
    theme.secondary = hsl(0.0, 0.0, 15.0); // #252526
    theme.sidebar = hsl(0.0, 0.0, 15.0);
    theme.title_bar = hsl(0.0, 0.0, 19.0);
    theme.title_bar_border = hsl(0.0, 0.0, 25.0);

    theme.border = hsl(0.0, 0.0, 25.0);
    theme.input = hsl(0.0, 0.0, 18.0);

    theme.foreground = hsl(0.0, 0.0, 80.0);
    theme.muted = hsl(0.0, 0.0, 22.0);
    theme.muted_foreground = hsl(0.0, 0.0, 55.0);
    theme.secondary_foreground = hsl(0.0, 0.0, 80.0);

    theme.danger = hsl(0.0, 75.0, 55.0);
    theme.danger_hover = hsl(0.0, 75.0, 60.0);
    theme.danger_active = hsl(0.0, 75.0, 48.0);
    theme.danger_foreground = hsl(0.0, 0.0, 100.0);

    theme.success = hsl(120.0, 50.0, 45.0);
    theme.success_hover = hsl(120.0, 50.0, 52.0);
    theme.success_active = hsl(120.0, 50.0, 38.0);
    theme.success_foreground = hsl(0.0, 0.0, 100.0);

    theme.info = accent;
    theme.info_hover = accent_hover;
    theme.info_active = accent_active;
    theme.info_foreground = hsl(0.0, 0.0, 100.0);

    theme.selection = accent.opacity(0.35);

    theme.popover = hsl(0.0, 0.0, 17.0);
    theme.popover_foreground = hsl(0.0, 0.0, 86.0);

    // 补全菜单匹配前缀高亮（暗色态：浅蓝在选中态深蓝 bg 上仍可见）
    theme.blue = hsl(207.0, 90.0, 70.0);
    theme.blue_light = hsl(207.0, 90.0, 80.0);
}

/// VSCode Light+ 配色
fn apply_light_palette(theme: &mut Theme) {
    let accent = hsl(207.0, 100.0, 38.0);
    let accent_hover = hsl(207.0, 100.0, 32.0);
    let accent_active = hsl(207.0, 100.0, 28.0);

    theme.accent = accent;
    theme.accent_foreground = hsl(0.0, 0.0, 100.0);
    theme.primary = accent;
    theme.primary_hover = accent_hover;
    theme.primary_active = accent_active;
    theme.primary_foreground = hsl(0.0, 0.0, 100.0);

    theme.link = accent;
    theme.link_hover = accent_hover;
    theme.link_active = accent_active;

    // 背景三段（VSCode Light）
    theme.background = hsl(0.0, 0.0, 100.0); // #FFFFFF
    theme.secondary = hsl(0.0, 0.0, 96.0); // #F3F3F3
    theme.sidebar = hsl(0.0, 0.0, 96.0);
    theme.title_bar = hsl(0.0, 0.0, 92.0);
    theme.title_bar_border = hsl(0.0, 0.0, 82.0);

    theme.border = hsl(0.0, 0.0, 85.0);
    theme.input = hsl(0.0, 0.0, 100.0);

    theme.foreground = hsl(0.0, 0.0, 12.0); // 近黑
    theme.muted = hsl(0.0, 0.0, 92.0);
    theme.muted_foreground = hsl(0.0, 0.0, 38.0);
    theme.secondary_foreground = hsl(0.0, 0.0, 12.0);

    theme.danger = hsl(0.0, 65.0, 48.0);
    theme.danger_hover = hsl(0.0, 65.0, 42.0);
    theme.danger_active = hsl(0.0, 65.0, 36.0);
    theme.danger_foreground = hsl(0.0, 0.0, 100.0);

    theme.success = hsl(120.0, 45.0, 35.0);
    theme.success_hover = hsl(120.0, 45.0, 30.0);
    theme.success_active = hsl(120.0, 45.0, 26.0);
    theme.success_foreground = hsl(0.0, 0.0, 100.0);

    theme.info = accent;
    theme.info_hover = accent_hover;
    theme.info_active = accent_active;
    theme.info_foreground = hsl(0.0, 0.0, 100.0);

    theme.selection = accent.opacity(0.20);

    theme.popover = hsl(0.0, 0.0, 100.0);
    theme.popover_foreground = hsl(0.0, 0.0, 12.0);

    // 补全菜单的匹配前缀高亮颜色（gpui-component 用 theme.blue 渲染）
    // 浅色态：选中项背景是 accent 深蓝，blue 必须比 accent 亮才能看清
    theme.blue = hsl(207.0, 100.0, 65.0);
    theme.blue_light = hsl(207.0, 100.0, 75.0);
}

/// HSL → Hsla：输入 0-360 / 0-100 / 0-100
fn hsl(h: f32, s: f32, l: f32) -> Hsla {
    hsla(h / 360.0, s / 100.0, l / 100.0, 1.0)
}

trait Opacity {
    fn opacity(self, alpha: f32) -> Self;
}

impl Opacity for Hsla {
    fn opacity(mut self, alpha: f32) -> Self {
        self.a = alpha.clamp(0.0, 1.0);
        self
    }
}
