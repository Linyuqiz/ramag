//! AssetSource：优先 ramag-ui 内嵌 svg（assets/icons），未命中回退 gpui_component_assets

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

/// 编译期内嵌 svg
#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
struct LocalAssets;

#[derive(Default, Clone, Copy)]
pub struct RamagAssets;

impl AssetSource for RamagAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }
        if let Some(file) = LocalAssets::get(path) {
            return Ok(Some(file.data));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut out: Vec<SharedString> = LocalAssets::iter()
            .filter_map(|p| p.starts_with(path).then(|| p.into()))
            .collect();
        if let Ok(upstream) = gpui_component_assets::Assets.list(path) {
            out.extend(upstream);
        }
        Ok(out)
    }
}
