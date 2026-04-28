//! 值显示增强模块
//!
//! 提供 Raw / JSON / Hex / base64 四种视图切换，以及 Gzip 自动解压
//! 仅作用于 String / Bytes 标量类型；List/Hash/Set/ZSet 容器类型不受影响

use base64::Engine as _;
use flate2::read::GzDecoder;
use std::io::Read;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// 原文（utf-8）
    #[default]
    Raw,
    /// JSON 解析 + pretty
    Json,
    /// Hex 字节流（每字节 2 位 + 空格分隔，每 16 字节换行）
    Hex,
    /// base64 编码
    Base64,
}

impl ViewMode {
    pub fn label(&self) -> &'static str {
        match self {
            ViewMode::Raw => "Raw",
            ViewMode::Json => "JSON",
            ViewMode::Hex => "Hex",
            ViewMode::Base64 => "base64",
        }
    }

    pub fn all() -> &'static [ViewMode] {
        &[
            ViewMode::Raw,
            ViewMode::Json,
            ViewMode::Hex,
            ViewMode::Base64,
        ]
    }
}

/// Gzip magic：检测到 `1f 8b` 前缀就尝试解压；失败返回 None
pub fn try_decompress_gzip(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() < 2 || bytes[0] != 0x1f || bytes[1] != 0x8b {
        return None;
    }
    let mut decoder = GzDecoder::new(bytes);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).ok()?;
    Some(out)
}

/// 以指定 ViewMode 渲染文本（Raw / JSON / Hex / base64）
pub fn render_text(text: &str, mode: ViewMode) -> String {
    match mode {
        ViewMode::Raw => text.to_string(),
        ViewMode::Json => pretty_json(text.as_bytes()),
        ViewMode::Hex => to_hex_dump(text.as_bytes()),
        ViewMode::Base64 => base64::engine::general_purpose::STANDARD.encode(text.as_bytes()),
    }
}

/// 以指定 ViewMode 渲染字节流
pub fn render_bytes(bytes: &[u8], mode: ViewMode) -> String {
    match mode {
        ViewMode::Raw => match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => format!("[{} bytes：非 UTF-8，请切到 Hex/base64 查看]", bytes.len()),
        },
        ViewMode::Json => pretty_json(bytes),
        ViewMode::Hex => to_hex_dump(bytes),
        ViewMode::Base64 => base64::engine::general_purpose::STANDARD.encode(bytes),
    }
}

/// 解析 bytes 为 JSON 并 pretty 输出；失败时返回原文 + 提示
fn pretty_json(bytes: &[u8]) -> String {
    match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(v) => match serde_json::to_string_pretty(&v) {
            Ok(s) => s,
            Err(_) => "(JSON 序列化失败)".to_string(),
        },
        Err(e) => {
            let preview = std::str::from_utf8(bytes).unwrap_or("（非 UTF-8）");
            format!("(无法解析为 JSON：{e})\n\n{preview}")
        }
    }
}

/// 经典 hex dump：每 16 字节一行；左侧偏移地址，右侧 ASCII 预览
fn to_hex_dump(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 4);
    for (i, chunk) in bytes.chunks(16).enumerate() {
        let offset = i * 16;
        out.push_str(&format!("{offset:08x}  "));
        // hex 部分（每字节 2 位 + 空格；不足 16 字节用空格补齐对齐）
        for (j, b) in chunk.iter().enumerate() {
            out.push_str(&format!("{b:02x} "));
            if j == 7 {
                out.push(' ');
            }
        }
        for j in chunk.len()..16 {
            out.push_str("   ");
            if j == 7 {
                out.push(' ');
            }
        }
        // ASCII 部分
        out.push_str(" |");
        for b in chunk {
            let c = if (0x20..0x7f).contains(b) {
                *b as char
            } else {
                '.'
            };
            out.push(c);
        }
        out.push_str("|\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gzip_detect_and_decompress() {
        // gzip 编码 "hello world"
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Write;
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(b"hello world").unwrap();
        let compressed = enc.finish().unwrap();

        let out = try_decompress_gzip(&compressed).unwrap();
        assert_eq!(&out, b"hello world");
    }

    #[test]
    fn gzip_non_gzip_returns_none() {
        assert!(try_decompress_gzip(b"not gzip").is_none());
        assert!(try_decompress_gzip(&[0x1f]).is_none()); // 太短
    }

    #[test]
    fn pretty_json_valid() {
        let out = pretty_json(br#"{"a":1,"b":[2,3]}"#);
        assert!(out.contains("\n  \"a\": 1"));
    }

    #[test]
    fn pretty_json_invalid_returns_preview() {
        let out = pretty_json(b"not json");
        assert!(out.contains("无法解析"));
        assert!(out.contains("not json"));
    }

    #[test]
    fn hex_dump_format() {
        let out = to_hex_dump(b"AB12");
        // "00000000  41 42 31 32                                       |AB12|"
        assert!(out.starts_with("00000000  41 42 31 32"));
        assert!(out.contains("|AB12|"));
    }

    #[test]
    fn render_text_modes() {
        assert_eq!(render_text("hi", ViewMode::Raw), "hi");
        assert_eq!(render_text("hi", ViewMode::Base64), "aGk=");
    }

    #[test]
    fn render_bytes_non_utf8_raw() {
        let s = render_bytes(&[0xff, 0xfe], ViewMode::Raw);
        assert!(s.contains("非 UTF-8"));
    }
}
