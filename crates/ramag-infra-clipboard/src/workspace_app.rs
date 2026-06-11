//! NSWorkspace FFI：前台应用信息 + 应用图标 PNG（来源标注）+ 打开链接 / Finder 显示

use cocoa::base::{id, nil};
use cocoa::foundation::NSArray;
use objc::{class, msg_send, sel, sel_impl};

use ramag_domain::entities::ClipSource;
use ramag_domain::error::{DomainError, Result};

use crate::pasteboard::{ns_string, tiff_to_png, to_rust_string};

pub(crate) fn frontmost_app() -> Option<ClipSource> {
    unsafe {
        let ws: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let app: id = msg_send![ws, frontmostApplication];
        if app == nil {
            return None;
        }
        let bundle_id = to_rust_string(msg_send![app, bundleIdentifier])?;
        let name =
            to_rust_string(msg_send![app, localizedName]).unwrap_or_else(|| bundle_id.clone());
        Some(ClipSource { bundle_id, name })
    }
}

/// 按 bundle_id 取应用图标并转 PNG。app 未安装 / 转码失败返回 None
pub(crate) fn app_icon_png(bundle_id: &str) -> Option<Vec<u8>> {
    unsafe {
        let ws: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let url: id = msg_send![ws, URLForApplicationWithBundleIdentifier: ns_string(bundle_id)];
        if url == nil {
            return None;
        }
        let path: id = msg_send![url, path];
        let icon: id = msg_send![ws, iconForFile: path];
        if icon == nil {
            return None;
        }
        let tiff: id = msg_send![icon, TIFFRepresentation];
        let (png, _dims) = tiff_to_png(tiff)?;
        Some(png)
    }
}

/// 用默认浏览器打开链接（NSWorkspace openURL）
pub(crate) fn open_url(url: &str) -> Result<()> {
    unsafe {
        let ns_url: id = msg_send![class!(NSURL), URLWithString: ns_string(url)];
        if ns_url == nil {
            return Err(DomainError::InvalidConfig(format!("无效链接：{url}")));
        }
        let ws: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let ok: bool = msg_send![ws, openURL: ns_url];
        if ok {
            Ok(())
        } else {
            Err(DomainError::Other(format!("打开链接失败：{url}")))
        }
    }
}

/// 在 Finder 中高亮显示文件（activateFileViewerSelectingURLs，多文件一并选中）
pub(crate) fn reveal_in_finder(paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    unsafe {
        let urls: Vec<id> = paths
            .iter()
            .map(|p| msg_send![class!(NSURL), fileURLWithPath: ns_string(p)])
            .collect();
        let arr = NSArray::arrayWithObjects(nil, &urls);
        let ws: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let _: () = msg_send![ws, activateFileViewerSelectingURLs: arr];
        Ok(())
    }
}
