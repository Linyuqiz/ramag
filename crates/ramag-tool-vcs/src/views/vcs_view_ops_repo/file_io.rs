//! 文件读盘 + 大文件截断 + 二进制识别 + 主线程 finalize

use super::{PF_FILE_MAX_BYTES, RawFileContent};
use crate::views::helpers::FileContentSnapshot;

/// 读盘失败（路径不存在 / 权限不足）→ raw.error 携带消息，UI 渲染层提示
pub(in crate::views) fn read_raw_file_content(abs: &std::path::Path, rel: &str) -> RawFileContent {
    let metadata = match std::fs::metadata(abs) {
        Ok(m) => m,
        Err(e) => {
            return RawFileContent {
                path: rel.to_string(),
                lines: Vec::new(),
                truncated: false,
                binary: false,
                error: Some(format!("无法访问文件: {e}")),
            };
        }
    };
    if !metadata.is_file() {
        return RawFileContent {
            path: rel.to_string(),
            lines: Vec::new(),
            truncated: false,
            binary: false,
            error: Some("不是普通文件（可能是软链接 / 设备文件）".into()),
        };
    }
    let total_size = metadata.len();
    let truncated = total_size > PF_FILE_MAX_BYTES;
    // 截断时仅读前 PF_FILE_MAX_BYTES 字节，避免一口气读 100MB 大 log
    let read_result = if truncated {
        read_first_bytes(abs, PF_FILE_MAX_BYTES as usize)
    } else {
        std::fs::read(abs)
    };
    let bytes = match read_result {
        Ok(b) => b,
        Err(e) => {
            return RawFileContent {
                path: rel.to_string(),
                lines: Vec::new(),
                truncated: false,
                binary: false,
                error: Some(format!("读取文件失败: {e}")),
            };
        }
    };
    // 二进制识别：前 8KB 任一字节为 NUL → 不渲染内容
    let head_len = bytes.len().min(8192);
    if bytes[..head_len].contains(&0) {
        return RawFileContent {
            path: rel.to_string(),
            lines: Vec::new(),
            truncated: false,
            binary: true,
            error: None,
        };
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
    RawFileContent {
        path: rel.to_string(),
        lines,
        truncated,
        binary: false,
        error: None,
    }
}

/// 主线程 finalize：算 max_chars + 包 Rc → FileContentSnapshot
pub(super) fn finalize_file_snapshot(raw: RawFileContent) -> FileContentSnapshot {
    let max_chars = raw
        .lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);
    FileContentSnapshot {
        path: raw.path,
        lines: std::rc::Rc::new(raw.lines),
        max_chars,
        truncated: raw.truncated,
        binary: raw.binary,
        error: raw.error,
    }
}

/// 读取文件前 `limit` 字节（用于大文件截断预览）
fn read_first_bytes(path: &std::path::Path, limit: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let file = std::fs::File::open(path)?;
    let mut buf = Vec::with_capacity(limit);
    file.take(limit as u64).read_to_end(&mut buf)?;
    Ok(buf)
}
