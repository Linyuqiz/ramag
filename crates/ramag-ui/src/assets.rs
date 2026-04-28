//! Ramag 自定义 AssetSource
//!
//! - 优先从 ramag-ui 自身嵌入的 svg 加载（icons 目录）
//! - 找不到时回退到上游 `gpui_component_assets::Assets`，让组件内部用到的
//!   通用 Lucide 图标（chevron-down、close、calendar 等）依然能渲染
//!
//! 这样 ramag 业务专属图标全部由本 crate 提供，不再依赖 fork 维护新增 svg。

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// 嵌入 ramag 项目的 svg 资源（编译期内嵌进二进制）
#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
struct LocalAssets;

/// 复合 AssetSource：先查本地，再回退到上游
#[derive(Default, Clone, Copy)]
pub struct RamagAssets;

impl AssetSource for RamagAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }
        // 1) 本地 svg 命中
        if let Some(file) = LocalAssets::get(path) {
            return Ok(Some(file.data));
        }
        // 2) 回退到上游内置图标
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        // 简单合并两套清单（重复无影响）
        let mut out: Vec<SharedString> = LocalAssets::iter()
            .filter_map(|p| p.starts_with(path).then(|| p.into()))
            .collect();
        if let Ok(upstream) = gpui_component_assets::Assets.list(path) {
            out.extend(upstream);
        }
        Ok(out)
    }
}
