#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
// objc 0.2 的 msg_send!/sel! 宏内嵌 cfg(feature="cargo-clippy")，在调用方 crate 触发该警告，按宏来源放行
#![allow(unexpected_cfgs)]

//! macOS 剪贴板驱动：NSPasteboard 轮询读写 / 来源应用标注 / 粘贴模拟 / 媒体缓存。
//! 与 Storage / Git 不同：方法全部为同步快调用且须在主线程（GPUI 前台 executor）使用，
//! 不需要 tokio，也不需要 std::thread 桥接（除 CGEvent 延迟发送）

mod hotkey;
mod media;
mod paste;
mod pasteboard;
mod workspace_app;

pub use hotkey::HotkeyListener;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use parking_lot::Mutex;

use ramag_domain::entities::{CapturedClip, ClipSource};
use ramag_domain::error::{DomainError, Result};
use ramag_domain::traits::ClipboardDriver;

pub struct MacClipboardDriver {
    /// 最近一次本应用写回产生的 changeCount，采集循环据此跳过自写回
    own_change: AtomicI64,
    /// 应用图标 PNG 缓存（None 也缓存，避免反复对未安装应用做查找）
    icon_cache: Mutex<HashMap<String, Option<Arc<Vec<u8>>>>>,
    media: media::MediaStore,
}

impl MacClipboardDriver {
    pub fn new() -> Self {
        Self {
            own_change: AtomicI64::new(-1),
            icon_cache: Mutex::new(HashMap::new()),
            media: media::MediaStore::new(),
        }
    }
}

impl Default for MacClipboardDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardDriver for MacClipboardDriver {
    fn change_count(&self) -> i64 {
        pasteboard::change_count()
    }

    fn own_change_count(&self) -> i64 {
        self.own_change.load(Ordering::Relaxed)
    }

    fn read(&self) -> Result<Option<CapturedClip>> {
        pasteboard::read()
    }

    fn write_text(&self, text: &str, rtf: Option<&[u8]>) -> Result<()> {
        let count = pasteboard::write_text(text, rtf)?;
        self.own_change.store(count, Ordering::Relaxed);
        Ok(())
    }

    fn write_image_png(&self, png: &[u8]) -> Result<()> {
        let count = pasteboard::write_image_png(png)?;
        self.own_change.store(count, Ordering::Relaxed);
        Ok(())
    }

    fn write_files(&self, paths: &[String]) -> Result<()> {
        let count = pasteboard::write_files(paths)?;
        self.own_change.store(count, Ordering::Relaxed);
        Ok(())
    }

    fn frontmost_app(&self) -> Option<ClipSource> {
        workspace_app::frontmost_app()
    }

    fn app_icon_png(&self, bundle_id: &str) -> Option<Arc<Vec<u8>>> {
        let mut cache = self.icon_cache.lock();
        if let Some(hit) = cache.get(bundle_id) {
            return hit.clone();
        }
        let icon = workspace_app::app_icon_png(bundle_id).map(Arc::new);
        cache.insert(bundle_id.to_string(), icon.clone());
        icon
    }

    fn persist_media(&self, key: &str, bytes: &[u8]) -> Result<String> {
        self.media.persist(key, bytes)
    }

    fn read_media(&self, path: &str) -> Result<Vec<u8>> {
        self.media.read(path)
    }

    fn list_media(&self) -> Result<Vec<String>> {
        self.media.list()
    }

    fn remove_media(&self, path: &str) -> Result<()> {
        self.media.remove(path)
    }

    fn accessibility_trusted(&self, prompt: bool) -> bool {
        paste::accessibility_trusted(prompt)
    }

    fn paste_to_app(&self, bundle_id: Option<&str>) -> Result<()> {
        if !paste::accessibility_trusted(false) {
            // 无权限：弹系统授权引导，用户可一键跳到「辅助功能」设置授权
            paste::accessibility_trusted(true);
            return Err(DomainError::Other(
                "需在「系统设置 › 隐私与安全性 › 辅助功能」勾选 Ramag 后才能自动粘贴；内容已复制，可手动 cmd-V".into(),
            ));
        }
        if let Some(bid) = bundle_id {
            // 激活失败不视为错误：目标可能已在前台
            let _ = paste::activate_app(bid);
        }
        // 抽屉用 Floating 抢前台做中文输入，粘贴需先切回原应用，故延迟略大
        paste::post_cmd_v_delayed(220);
        Ok(())
    }

    fn open_url(&self, url: &str) -> Result<()> {
        workspace_app::open_url(url)
    }

    fn reveal_in_finder(&self, paths: &[String]) -> Result<()> {
        workspace_app::reveal_in_finder(paths)
    }

    fn paths_exist(&self, paths: &[String]) -> bool {
        !paths.is_empty() && paths.iter().all(|p| std::path::Path::new(p).exists())
    }
}
