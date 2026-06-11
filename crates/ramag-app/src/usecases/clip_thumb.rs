//! 剪贴图片缩略图生成：解码 → 按最大宽度等比缩放 → 编码 PNG。
//! 列表展示用缩略图（小图）降低"解密 + 解码"成本，原图仅详情/复制时加载

use std::io::Cursor;

use image::ImageFormat;
use ramag_domain::error::{DomainError, Result};

/// 缩略图最大宽度（高度等比，已小于此宽则不放大）
pub const THUMB_MAX_W: u32 = 320;

/// 生成缩略图 PNG。解码失败（非图片 / 损坏）返回 Err，由调用方降级为无缩略图
pub fn make_thumbnail(png: &[u8], max_w: u32) -> Result<Vec<u8>> {
    let img = image::load_from_memory(png)
        .map_err(|e| DomainError::Other(format!("缩略图解码失败：{e}")))?;
    let (w, h) = (img.width(), img.height());
    let scaled = if w > max_w {
        let nh = (h.saturating_mul(max_w) / w.max(1)).max(1);
        img.thumbnail(max_w, nh)
    } else {
        img
    };
    let mut out = Vec::new();
    scaled
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .map_err(|e| DomainError::Other(format!("缩略图编码失败：{e}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一张纯色 PNG（宽 w 高 h）
    fn solid_png(w: u32, h: u32) -> Vec<u8> {
        let buf = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(buf)
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn large_image_downscaled_keeping_ratio() {
        let png = solid_png(800, 400);
        let thumb = make_thumbnail(&png, 320).unwrap();
        let decoded = image::load_from_memory(&thumb).unwrap();
        assert_eq!(decoded.width(), 320);
        assert_eq!(decoded.height(), 160); // 800:400 = 320:160
        assert!(thumb.len() < png.len());
    }

    #[test]
    fn small_image_not_upscaled() {
        let png = solid_png(100, 80);
        let thumb = make_thumbnail(&png, 320).unwrap();
        let decoded = image::load_from_memory(&thumb).unwrap();
        assert_eq!(decoded.width(), 100);
        assert_eq!(decoded.height(), 80);
    }

    #[test]
    fn non_image_bytes_errs() {
        assert!(make_thumbnail(b"not an image", 320).is_err());
    }
}
