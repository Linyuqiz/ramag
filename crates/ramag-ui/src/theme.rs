//! VSCode 风格暗 / 亮主题。`init_theme` 启动时初始化，ActivityBar 太阳/月亮按钮切换

use std::sync::Arc;

use gpui::{App, Global, Hsla, WindowAppearance, hsla};
use gpui_component::{Theme, ThemeMode};
use ramag_domain::traits::Storage;

/// 让 UI 层切主题时访问 Storage 做持久化
pub struct StorageGlobal(pub Arc<dyn Storage>);
impl Global for StorageGlobal {}

/// main 可能没注入，None 时不持久化
pub fn storage_from_cx(cx: &App) -> Option<Arc<dyn Storage>> {
    cx.try_global::<StorageGlobal>().map(|g| g.0.clone())
}

/// 跟随系统时系统外观变化会自动同步；用户显式选过 dark/light 后忽略
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

pub fn mode_from_appearance(appearance: WindowAppearance) -> Mode {
    match appearance {
        WindowAppearance::Dark | WindowAppearance::VibrantDark => Mode::Dark,
        _ => Mode::Light,
    }
}

/// preference None/"system" 跟随系统，"dark"/"light" 用户固定
pub fn init_theme(preference: Option<&str>, appearance: WindowAppearance, cx: &mut App) {
    let (mode, follow) = match preference {
        Some("dark") => (Mode::Dark, false),
        Some("light") => (Mode::Light, false),
        _ => (mode_from_appearance(appearance), true),
    };
    apply_theme(mode, cx);
    set_following_system(cx, follow);
}

/// 仅 follow_system=true 时重应用，用户显式选过的不动
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

pub fn apply_theme(mode: Mode, cx: &mut App) {
    match mode {
        Mode::Dark => {
            Theme::change(ThemeMode::Dark, None, cx);
            apply_dark_palette(Theme::global_mut(cx));
        }
        Mode::Light => {
            Theme::change(ThemeMode::Light, None, cx);
            apply_light_palette(Theme::global_mut(cx));
        }
    }
    // 命令编辑器背景对齐主背景：gpui 默认 editor.background 是纯黑，浮在主背景上显突兀
    unify_editor_background(cx);
}

/// 命令编辑器（code_editor）背景对齐主背景：gpui 默认 highlight 的 editor.background
/// 是纯黑 #0a0a0a，浮在 ramag 主背景（深灰 / 白）上很突兀；这里只改背景、保留语法配色
fn unify_editor_background(cx: &mut App) {
    let theme = Theme::global_mut(cx);
    let bg = theme.background;
    if theme.highlight_theme.style.editor_background == Some(bg) {
        return;
    }
    let mut hl = (*theme.highlight_theme).clone();
    hl.style.editor_background = Some(bg);
    theme.highlight_theme = Arc::new(hl);
}

pub fn current_mode(cx: &App) -> Mode {
    if matches!(Theme::global(cx).mode, ThemeMode::Light) {
        Mode::Light
    } else {
        Mode::Dark
    }
}

/// VSCode Dark+ 配色
fn apply_dark_palette(theme: &mut Theme) {
    // VSCode 蓝（#007ACC）
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

    // 补全前缀高亮：暗色下浅蓝可见于选中态深蓝 bg
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

    theme.background = hsl(0.0, 0.0, 100.0); // #FFFFFF
    theme.secondary = hsl(0.0, 0.0, 96.0); // #F3F3F3
    theme.sidebar = hsl(0.0, 0.0, 96.0);
    theme.title_bar = hsl(0.0, 0.0, 92.0);
    theme.title_bar_border = hsl(0.0, 0.0, 82.0);

    theme.border = hsl(0.0, 0.0, 85.0);
    theme.input = hsl(0.0, 0.0, 100.0);

    theme.foreground = hsl(0.0, 0.0, 12.0);
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

    // 补全前缀高亮：浅色下 blue 须比 accent 更亮才能看清
    theme.blue = hsl(207.0, 100.0, 65.0);
    theme.blue_light = hsl(207.0, 100.0, 75.0);
}

/// 0-360 / 0-100 / 0-100 → Hsla
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

#[cfg(test)]
mod tests {
    use super::*;

    /// ActivityBar 太阳/月亮按钮靠 toggled 在暗 / 亮间互切
    #[test]
    fn mode_toggle_roundtrip() {
        assert_eq!(Mode::Dark.toggled(), Mode::Light);
        assert_eq!(Mode::Light.toggled(), Mode::Dark);
    }
}
