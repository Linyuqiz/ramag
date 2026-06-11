//! NSPasteboard FFI：changeCount 轮询 / 多类型读取 / 隐私标记检测 / 写回。
//! 全部调用须在主线程（AppKit 约定，autorelease 依赖主 RunLoop 池）

use std::ffi::CStr;

use cocoa::appkit::{
    NSFilenamesPboardType, NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeString,
    NSPasteboardTypeTIFF,
};
use cocoa::base::{id, nil};
use cocoa::foundation::{NSArray, NSData, NSString};
use objc::{class, msg_send, sel, sel_impl};

use ramag_domain::entities::CapturedClip;
use ramag_domain::error::{DomainError, Result};

/// nspasteboard.org 约定：带这些标记的内容不应被剪贴板管理器记录
const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";
const TRANSIENT_TYPE: &str = "org.nspasteboard.TransientType";
/// NSPasteboardTypeRTF
const RTF_TYPE: &str = "public.rtf";

/// autorelease 交主 RunLoop 池回收
pub(crate) unsafe fn ns_string(s: &str) -> id {
    unsafe {
        let ns: id = NSString::alloc(nil).init_str(s);
        msg_send![ns, autorelease]
    }
}

pub(crate) unsafe fn to_rust_string(ns: id) -> Option<String> {
    if ns == nil {
        return None;
    }
    unsafe {
        let ptr = NSString::UTF8String(ns);
        if ptr.is_null() {
            return None;
        }
        Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
    }
}

unsafe fn data_to_vec(data: id) -> Option<Vec<u8>> {
    if data == nil {
        return None;
    }
    unsafe {
        let len = NSData::length(data) as usize;
        let bytes = NSData::bytes(data);
        if bytes.is_null() || len == 0 {
            return None;
        }
        Some(std::slice::from_raw_parts(bytes.cast::<u8>(), len).to_vec())
    }
}

unsafe fn ns_data(bytes: &[u8]) -> id {
    unsafe {
        msg_send![class!(NSData),
            dataWithBytes: bytes.as_ptr().cast::<std::ffi::c_void>()
            length: bytes.len() as u64]
    }
}

fn general() -> id {
    unsafe { NSPasteboard::generalPasteboard(nil) }
}

pub(crate) fn change_count() -> i64 {
    unsafe { general().changeCount() }
}

unsafe fn has_type(pb: id, type_name: &str) -> bool {
    unsafe {
        let types: id = msg_send![pb, types];
        if types == nil {
            return false;
        }
        let needle = ns_string(type_name);
        msg_send![types, containsObject: needle]
    }
}

/// PNG IHDR 固定偏移取宽高
fn png_dims(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || png.get(1..4)? != b"PNG" {
        return None;
    }
    let w = u32::from_be_bytes(png.get(16..20)?.try_into().ok()?);
    let h = u32::from_be_bytes(png.get(20..24)?.try_into().ok()?);
    Some((w, h))
}

/// TIFF NSData → (PNG bytes, 尺寸)。NSBitmapImageFileTypePNG = 4
pub(crate) unsafe fn tiff_to_png(tiff: id) -> Option<(Vec<u8>, (u32, u32))> {
    if tiff == nil {
        return None;
    }
    unsafe {
        let rep: id = msg_send![class!(NSBitmapImageRep), imageRepWithData: tiff];
        if rep == nil {
            return None;
        }
        let w: i64 = msg_send![rep, pixelsWide];
        let h: i64 = msg_send![rep, pixelsHigh];
        let png: id = msg_send![rep, representationUsingType: 4u64 properties: nil];
        let bytes = data_to_vec(png)?;
        Some((bytes, (w.max(0) as u32, h.max(0) as u32)))
    }
}

/// 读当前剪贴板。优先级：隐私标记 > 文件 > 图片 > 文本（与 Paste 等管理器一致）
pub(crate) fn read() -> Result<Option<CapturedClip>> {
    unsafe {
        let pb = general();
        if has_type(pb, CONCEALED_TYPE) || has_type(pb, TRANSIENT_TYPE) {
            return Ok(Some(CapturedClip {
                concealed: true,
                ..Default::default()
            }));
        }

        let mut cap = CapturedClip::default();

        // Finder 复制文件：同时带字符串表示，文件语义优先
        let plist: id = pb.propertyListForType(NSFilenamesPboardType);
        if plist != nil {
            let count = NSArray::count(plist);
            for i in 0..count {
                let item = NSArray::objectAtIndex(plist, i);
                if let Some(p) = to_rust_string(item) {
                    cap.files.push(p);
                }
            }
        }
        if !cap.files.is_empty() {
            return Ok(Some(cap));
        }

        // 图片：PNG 直读；只有 TIFF（截图常见）则转码
        if let Some(bytes) = data_to_vec(pb.dataForType(NSPasteboardTypePNG)) {
            cap.image_dims = png_dims(&bytes);
            cap.image_png = Some(bytes);
            return Ok(Some(cap));
        }
        let tiff: id = pb.dataForType(NSPasteboardTypeTIFF);
        if tiff != nil
            && let Some((bytes, dims)) = tiff_to_png(tiff)
        {
            cap.image_dims = Some(dims);
            cap.image_png = Some(bytes);
            return Ok(Some(cap));
        }

        // 文本（可附带 RTF 富文本表示）
        if let Some(text) = to_rust_string(pb.stringForType(NSPasteboardTypeString)) {
            cap.rtf = data_to_vec(pb.dataForType(ns_string(RTF_TYPE)));
            cap.text = Some(text);
            return Ok(Some(cap));
        }

        Ok(None)
    }
}

/// 写文本（可带 RTF）。返回写后 changeCount（自写回抑制用）
pub(crate) fn write_text(text: &str, rtf: Option<&[u8]>) -> Result<i64> {
    unsafe {
        let pb = general();
        let _: i64 = pb.clearContents();
        let ok: bool = msg_send![pb, setString: ns_string(text) forType: NSPasteboardTypeString];
        if !ok {
            return Err(DomainError::Other("写文本到剪贴板失败".into()));
        }
        if let Some(bytes) = rtf {
            let _: bool = msg_send![pb, setData: ns_data(bytes) forType: ns_string(RTF_TYPE)];
        }
        Ok(pb.changeCount())
    }
}

pub(crate) fn write_image_png(png: &[u8]) -> Result<i64> {
    unsafe {
        let pb = general();
        let _: i64 = pb.clearContents();
        let ok: bool = msg_send![pb, setData: ns_data(png) forType: NSPasteboardTypePNG];
        if !ok {
            return Err(DomainError::Other("写图片到剪贴板失败".into()));
        }
        Ok(pb.changeCount())
    }
}

pub(crate) fn write_files(paths: &[String]) -> Result<i64> {
    unsafe {
        let pb = general();
        let ns_paths: Vec<id> = paths.iter().map(|p| ns_string(p)).collect();
        let arr = NSArray::arrayWithObjects(nil, &ns_paths);
        let types = NSArray::arrayWithObject(nil, NSFilenamesPboardType);
        let _: i64 = pb.declareTypes_owner(types, nil);
        let ok: bool = msg_send![pb, setPropertyList: arr forType: NSFilenamesPboardType];
        if !ok {
            return Err(DomainError::Other("写文件列表到剪贴板失败".into()));
        }
        Ok(pb.changeCount())
    }
}

#[cfg(test)]
mod tests {
    use super::png_dims;

    #[test]
    fn png_header_dims() {
        // 最小 PNG 头：签名 + IHDR 长度/类型 + 宽 800 高 600
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        png.extend_from_slice(&[0, 0, 0, 13]);
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&800u32.to_be_bytes());
        png.extend_from_slice(&600u32.to_be_bytes());
        assert_eq!(png_dims(&png), Some((800, 600)));
        assert_eq!(png_dims(b"not a png"), None);
    }
}
