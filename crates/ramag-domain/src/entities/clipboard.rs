//! 剪贴板历史实体：条目 / 类型 / 设置 / 采集原始数据 + 分类与指纹纯函数

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 剪贴条目唯一标识
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClipId(pub Uuid);

impl ClipId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ClipId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ClipId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 条目类型。Text/Link/Color 互斥（由 classify_text 决定）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClipKind {
    Text,
    Link,
    Color,
    Image,
    Files,
}

impl ClipKind {
    pub fn label(&self) -> &'static str {
        match self {
            ClipKind::Text => "文本",
            ClipKind::Link => "链接",
            ClipKind::Color => "颜色",
            ClipKind::Image => "图片",
            ClipKind::Files => "文件",
        }
    }

    /// 英文类型名（卡片标题条用，对齐 Paste 风格）
    pub fn label_en(&self) -> &'static str {
        match self {
            ClipKind::Text => "Text",
            ClipKind::Link => "Link",
            ClipKind::Color => "Color",
            ClipKind::Image => "Image",
            ClipKind::Files => "Files",
        }
    }

    /// 全部枚举值（UI 筛选器用）
    pub fn all() -> &'static [ClipKind] {
        &[
            ClipKind::Text,
            ClipKind::Link,
            ClipKind::Color,
            ClipKind::Image,
            ClipKind::Files,
        ]
    }
}

/// 来源应用
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipSource {
    pub bundle_id: String,
    pub name: String,
}

/// 剪贴条目。文本内容直接入库（加密），图片落盘只存路径
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipItem {
    pub id: ClipId,
    pub kind: ClipKind,
    /// Text/Link/Color 的内容；Image/Files 为 None
    pub text: Option<String>,
    /// 富文本 RTF 原始数据（伴随 text；粘贴时与纯文本一起写回）
    #[serde(default)]
    pub rtf: Option<Vec<u8>>,
    /// Image 类型的原图落盘路径（AES 加密密文）
    pub image_path: Option<String>,
    /// Image 类型的缩略图落盘路径（AES 加密密文，列表展示用，降解码成本）
    #[serde(default)]
    pub thumb_path: Option<String>,
    /// 图片尺寸（宽, 高）
    pub image_dims: Option<(u32, u32)>,
    /// Files 类型的路径列表
    #[serde(default)]
    pub files: Vec<String>,
    /// 列表预览摘要
    pub preview: String,
    pub source: Option<ClipSource>,
    /// 原始内容字节数（文本字节 / PNG 字节）
    pub byte_size: u64,
    /// 内容指纹（fnv1a 十六进制），同内容去重
    pub content_hash: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
}

/// 采集与展示设置（prefs KV 以 JSON 持久化）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipboardSettings {
    /// 总开关：false 暂停记录
    pub enabled: bool,
    pub capture_images: bool,
    /// 历史条数上限
    pub max_items: u32,
    /// 保留天数
    pub max_age_days: u32,
    /// 单条内容字节上限，超出跳过不记录
    pub max_item_bytes: u64,
    /// 来源应用黑名单（bundle id）
    pub blacklist: Vec<String>,
    /// 抽屉选中后自动粘贴（需辅助功能权限；false 仅复制）
    pub auto_paste: bool,
}

impl Default for ClipboardSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            capture_images: true,
            max_items: 10000000,
            max_age_days: 360,
            max_item_bytes: 10 * 1024 * 1024,
            blacklist: Vec::new(),
            auto_paste: true,
        }
    }
}

/// 驱动读到的原始采集内容（未分类、未落库）
#[derive(Debug, Clone, Default)]
pub struct CapturedClip {
    pub text: Option<String>,
    pub rtf: Option<Vec<u8>>,
    pub image_png: Option<Vec<u8>>,
    pub image_dims: Option<(u32, u32)>,
    pub files: Vec<String>,
    /// 带 org.nspasteboard Concealed/Transient 标记（密码管理器等），不应记录
    pub concealed: bool,
}

/// fnv1a-64 内容指纹。std Hasher 不保证跨编译器版本稳定，落盘指纹必须自实现
pub fn fnv1a_hash(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// 文本二次分类：URL → Link、颜色字面量 → Color、否则 Text
pub fn classify_text(s: &str) -> ClipKind {
    let t = s.trim();
    if is_url(t) {
        ClipKind::Link
    } else if is_color(t) {
        ClipKind::Color
    } else {
        ClipKind::Text
    }
}

fn is_url(t: &str) -> bool {
    if t.contains(char::is_whitespace) {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://")) && t.len() > 10
}

fn is_color(t: &str) -> bool {
    // #RGB / #RGBA / #RRGGBB / #RRGGBBAA
    if let Some(hex) = t.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    // rgb(...) / rgba(...) / hsl(...) / hsla(...)
    let lower = t.to_ascii_lowercase();
    for prefix in ["rgb(", "rgba(", "hsl(", "hsla("] {
        if lower.starts_with(prefix) && lower.ends_with(')') {
            return true;
        }
    }
    false
}

/// 颜色文本 → RGB。仅支持 #hex 形态（UI 色卡预览用），其余返回 None
pub fn parse_hex_color(t: &str) -> Option<(u8, u8, u8)> {
    let hex = t.trim().strip_prefix('#')?;
    let expand = |c: char| -> Option<u8> {
        let v = c.to_digit(16)? as u8;
        Some(v << 4 | v)
    };
    let byte_at = |i: usize| -> Option<u8> { u8::from_str_radix(hex.get(i..i + 2)?, 16).ok() };
    match hex.len() {
        3 | 4 => {
            let mut cs = hex.chars();
            Some((
                expand(cs.next()?)?,
                expand(cs.next()?)?,
                expand(cs.next()?)?,
            ))
        }
        6 | 8 => Some((byte_at(0)?, byte_at(2)?, byte_at(4)?)),
        _ => None,
    }
}

/// 列表预览摘要：文本取首行截断，图片报尺寸，文件报名字
pub fn make_preview(
    kind: ClipKind,
    text: Option<&str>,
    files: &[String],
    dims: Option<(u32, u32)>,
) -> String {
    const MAX: usize = 120;
    match kind {
        ClipKind::Image => match dims {
            Some((w, h)) => format!("图片 {w}×{h}"),
            None => "图片".to_string(),
        },
        ClipKind::Files => match files {
            [] => "文件".to_string(),
            [one] => file_name(one),
            [first, ..] => format!("{} 等 {} 个文件", file_name(first), files.len()),
        },
        _ => {
            let line = text
                .unwrap_or_default()
                .trim()
                .lines()
                .next()
                .unwrap_or_default();
            if line.chars().count() > MAX {
                let cut: String = line.chars().take(MAX).collect();
                format!("{cut}…")
            } else {
                line.to_string()
            }
        }
    }
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_hash_stable_known_values() {
        // 与 FNV-1a 参考值一致，保证跨版本稳定
        assert_eq!(fnv1a_hash(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a_hash(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a_hash(b"hello"), fnv1a_hash(b"hello"));
        assert_ne!(fnv1a_hash(b"hello"), fnv1a_hash(b"hellp"));
    }

    #[test]
    fn classify_url_color_text() {
        assert_eq!(classify_text("https://example.com/page"), ClipKind::Link);
        assert_eq!(classify_text("  http://a.cn/x  "), ClipKind::Link);
        assert_eq!(classify_text("https:// broken url"), ClipKind::Text);
        assert_eq!(classify_text("#ff8800"), ClipKind::Color);
        assert_eq!(classify_text("#F80"), ClipKind::Color);
        assert_eq!(classify_text("#ff880042"), ClipKind::Color);
        assert_eq!(classify_text("#ggg"), ClipKind::Text);
        assert_eq!(classify_text("rgb(1, 2, 3)"), ClipKind::Color);
        assert_eq!(classify_text("hsla(0, 0%, 0%, 1)"), ClipKind::Color);
        assert_eq!(classify_text("SELECT 1;"), ClipKind::Text);
    }

    #[test]
    fn hex_color_parses() {
        assert_eq!(parse_hex_color("#ff8800"), Some((0xff, 0x88, 0x00)));
        assert_eq!(parse_hex_color("#f80"), Some((0xff, 0x88, 0x00)));
        assert_eq!(parse_hex_color("#ff880042"), Some((0xff, 0x88, 0x00)));
        assert_eq!(parse_hex_color("rgb(1,2,3)"), None);
    }

    #[test]
    fn preview_truncates_and_describes() {
        let long = "x".repeat(200);
        let p = make_preview(ClipKind::Text, Some(&long), &[], None);
        assert_eq!(p.chars().count(), 121);
        assert!(p.ends_with('…'));

        let multi = "first line\nsecond";
        assert_eq!(
            make_preview(ClipKind::Text, Some(multi), &[], None),
            "first line"
        );

        assert_eq!(
            make_preview(ClipKind::Image, None, &[], Some((800, 600))),
            "图片 800×600"
        );

        let files = vec!["/tmp/a.txt".to_string(), "/tmp/b.txt".to_string()];
        assert_eq!(
            make_preview(ClipKind::Files, None, &files, None),
            "a.txt 等 2 个文件"
        );
        let one = vec!["/tmp/solo.png".to_string()];
        assert_eq!(make_preview(ClipKind::Files, None, &one, None), "solo.png");
    }
}
