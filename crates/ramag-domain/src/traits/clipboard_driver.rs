//! 剪贴板驱动 trait：macOS 实现在 ramag-infra-clipboard。
//! 与 Driver / KvDriver 等不同：方法全部为同步快调用（无网络 IO），
//! 且按 AppKit 约定须在主线程调用（GPUI 前台 executor 即主线程）

use std::sync::Arc;

use crate::entities::{CapturedClip, ClipSource};
use crate::error::Result;

pub trait ClipboardDriver: Send + Sync {
    /// 系统剪贴板修改计数（NSPasteboard.changeCount），轮询比对用
    fn change_count(&self) -> i64;

    /// 最近一次由本应用写回产生的 changeCount，自写回抑制用
    fn own_change_count(&self) -> i64;

    /// 读取当前剪贴板。空剪贴板或无可识别类型返回 None
    fn read(&self) -> Result<Option<CapturedClip>>;

    /// 写文本回剪贴板（可附带 RTF 富文本表示）
    fn write_text(&self, text: &str, rtf: Option<&[u8]>) -> Result<()>;

    /// 写 PNG 图片回剪贴板
    fn write_image_png(&self, png: &[u8]) -> Result<()>;

    /// 写文件路径列表回剪贴板
    fn write_files(&self, paths: &[String]) -> Result<()>;

    /// 当前前台应用（采集来源标注）
    fn frontmost_app(&self) -> Option<ClipSource>;

    /// 应用图标 PNG（实现内部按 bundle_id 缓存）；取不到返回 None
    fn app_icon_png(&self, bundle_id: &str) -> Option<Arc<Vec<u8>>>;

    /// 字节落盘到媒体缓存（key 形如 `{hash}.img` / `{hash}.thumb`，同名去重），返回路径。
    /// 加密由上层 service 负责，此处只写原始字节
    fn persist_media(&self, key: &str, bytes: &[u8]) -> Result<String>;

    /// 读媒体文件原始字节（密文，由 service 解密）
    fn read_media(&self, path: &str) -> Result<Vec<u8>>;

    /// 列出媒体缓存目录内全部文件路径（孤儿清理用）
    fn list_media(&self) -> Result<Vec<String>>;

    /// 删除落盘媒体文件（容忍文件不存在）
    fn remove_media(&self, path: &str) -> Result<()>;

    /// 辅助功能权限是否已授予；prompt=true 时弹系统授权引导
    fn accessibility_trusted(&self, prompt: bool) -> bool;

    /// 激活指定应用（None 跳过激活）并模拟 cmd-V 粘贴。需辅助功能权限
    fn paste_to_app(&self, bundle_id: Option<&str>) -> Result<()>;

    /// 用默认浏览器打开链接
    fn open_url(&self, url: &str) -> Result<()>;

    /// 在 Finder 中显示文件（多文件高亮选中）
    fn reveal_in_finder(&self, paths: &[String]) -> Result<()>;

    /// 文件路径是否仍存在（粘贴前失效校验）
    fn paths_exist(&self, paths: &[String]) -> bool;
}
