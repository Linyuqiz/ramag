//! ClipboardService：剪贴板采集 + 历史聚合。与 ConnectionService 并列，共用同一份 redb。
//! 采集判定（去重 / 黑名单 / 大小 / 分类）抽成纯函数 `decide_capture` 便于测试，
//! `capture_tick` 仅做 driver/storage 编排

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::Utc;
use parking_lot::RwLock;
use ramag_domain::entities::{
    CapturedClip, ClipId, ClipItem, ClipKind, ClipSource, ClipboardSettings, classify_text,
    fnv1a_hash, make_preview,
};
use ramag_domain::error::{DomainError, Result};
use ramag_domain::traits::{ClipboardDriver, Storage};
use tracing::{debug, warn};

use crate::usecases::clip_thumb::{THUMB_MAX_W, make_thumbnail};

/// 设置持久化 key（prefs 表，JSON）
const SETTINGS_KEY: &str = "clipboard_settings";

/// 历史清理上限（固定策略，不开放设置）：最多 100 万条 / 360 天，超出在每次入库后清理最旧
const MAX_ITEMS: u32 = 1_000_000;
const MAX_AGE_DAYS: u32 = 360;

/// 内存缓存窗口：常驻最近 N 条（已解密），视图唤起 / 刷新同步读；内存与历史总量解耦
const CACHE_WINDOW: usize = 10_000;

/// 采集判定结果（纯逻辑产物，不触 IO）
#[derive(Debug, PartialEq)]
pub enum CaptureDecision {
    /// 跳过：隐私标记 / 黑名单 / 超限 / 空内容
    Skip(&'static str),
    /// 内容指纹（用于查重）+ 待入库条目骨架（image_path 由编排层补）
    Record { hash: String, kind: ClipKind },
}

pub struct ClipboardService {
    driver: Arc<dyn ClipboardDriver>,
    storage: Arc<dyn Storage>,
    /// 历史变更版本号：任何写操作（采集 / 复制 / 删除）后自增。
    /// 视图轮询此值，仅在变化时才重载解密，避免每拍全表解密
    revision: Arc<AtomicU64>,
    /// 已解密的最近 N 条窗口缓存（最近优先）。写操作增量维护，视图同步快照取
    cache: Arc<RwLock<Vec<ClipItem>>>,
}

impl ClipboardService {
    pub fn new(driver: Arc<dyn ClipboardDriver>, storage: Arc<dyn Storage>) -> Self {
        Self {
            driver,
            storage,
            revision: Arc::new(AtomicU64::new(0)),
            cache: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn driver(&self) -> &Arc<dyn ClipboardDriver> {
        &self.driver
    }

    /// 当前历史版本号（视图据此判断是否需要重载）
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    fn bump(&self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
    }

    // —— 内存窗口缓存 ——

    /// 启动预热：解密最近 CACHE_WINDOW 条入缓存（仅启动调一次）
    pub async fn preload(&self) {
        match self.storage.clip_list_recent(CACHE_WINDOW).await {
            Ok(items) => {
                *self.cache.write() = items;
                self.bump();
            }
            Err(e) => warn!(error = %e, "clip cache preload failed"),
        }
    }

    /// 同步取缓存快照（已解密、最近优先）。视图唤起 / 刷新用，无 IO、无解密
    pub fn cached_snapshot(&self) -> Vec<ClipItem> {
        self.cache.read().clone()
    }

    /// 缓存增量更新：移除旧同 id → 插最前 → 去超龄 + 截窗口（全内存，不解密）
    fn cache_upsert(&self, item: ClipItem) {
        let cutoff = Utc::now() - chrono::Duration::days(i64::from(MAX_AGE_DAYS));
        let mut c = self.cache.write();
        c.retain(|i| i.id != item.id);
        c.insert(0, item);
        c.retain(|i| i.last_used_at >= cutoff);
        if c.len() > CACHE_WINDOW {
            c.truncate(CACHE_WINDOW);
        }
    }

    fn cache_remove(&self, id: &ClipId) {
        self.cache.write().retain(|i| &i.id != id);
    }

    fn cache_clear(&self) {
        self.cache.write().clear();
    }

    // —— 设置 ——

    pub async fn load_settings(&self) -> ClipboardSettings {
        match self.storage.get_preference(SETTINGS_KEY).await {
            Ok(Some(json)) => serde_json::from_str(&json).unwrap_or_default(),
            _ => ClipboardSettings::default(),
        }
    }

    pub async fn save_settings(&self, settings: &ClipboardSettings) -> Result<()> {
        let json = serde_json::to_string(settings)
            .map_err(|e| DomainError::Storage(format!("序列化剪贴设置失败：{e}")))?;
        self.storage.set_preference(SETTINGS_KEY, &json).await
    }

    // —— 采集 ——

    /// 轮询一拍：changeCount 变化时读取并按设置决定是否入库。
    /// 返回 true 表示历史有变更（UI 需刷新）
    pub async fn capture_tick(&self, settings: &ClipboardSettings) -> Result<bool> {
        if !settings.enabled {
            return Ok(false);
        }
        let count = self.driver.change_count();
        // 自写回产生的变更跳过（避免复制回剪贴板又记一遍）
        if count == self.driver.own_change_count() {
            return Ok(false);
        }
        let Some(captured) = self.driver.read()? else {
            return Ok(false);
        };
        let source = self.driver.frontmost_app();

        match decide_capture(&captured, settings, source.as_ref()) {
            CaptureDecision::Skip(reason) => {
                debug!(reason, "clip capture skipped");
                Ok(false)
            }
            CaptureDecision::Record { hash, kind } => {
                self.record(captured, kind, hash, source, settings).await
            }
        }
    }

    /// 入库：命中指纹则提升旧条目，否则新建（图片先落盘）
    async fn record(
        &self,
        captured: CapturedClip,
        kind: ClipKind,
        hash: String,
        source: Option<ClipSource>,
        settings: &ClipboardSettings,
    ) -> Result<bool> {
        let now = Utc::now();
        if let Some(mut existing) = self.storage.clip_find_by_hash(&hash).await? {
            existing.last_used_at = now;
            if let Some(src) = source {
                existing.source = Some(src);
            }
            self.storage.clip_save(&existing).await?;
            self.cache_upsert(existing);
            self.bump();
            return Ok(true);
        }

        // 图片：原图 + 缩略图均加密落盘（密文）。缩略图生成失败降级为无缩略图
        let (image_path, thumb_path) = match (&captured.image_png, settings.capture_images) {
            (Some(png), true) => {
                let enc_full = self.storage.seal(png).await?;
                let full = self
                    .driver
                    .persist_media(&format!("{hash}.img"), &enc_full)?;
                let thumb = match make_thumbnail(png, THUMB_MAX_W) {
                    Ok(t) => {
                        let enc_t = self.storage.seal(&t).await?;
                        Some(
                            self.driver
                                .persist_media(&format!("{hash}.thumb"), &enc_t)?,
                        )
                    }
                    Err(e) => {
                        warn!(error = %e, "thumbnail generate failed");
                        None
                    }
                };
                (Some(full), thumb)
            }
            _ => (None, None),
        };
        let byte_size = captured
            .image_png
            .as_ref()
            .map(|p| p.len() as u64)
            .or_else(|| captured.text.as_ref().map(|t| t.len() as u64))
            .unwrap_or(0);
        let preview = make_preview(
            kind,
            captured.text.as_deref(),
            &captured.files,
            captured.image_dims,
        );

        let item = ClipItem {
            id: ClipId::new(),
            kind,
            text: captured.text,
            rtf: captured.rtf,
            image_path,
            thumb_path,
            image_dims: captured.image_dims,
            files: captured.files,
            preview,
            source,
            byte_size,
            content_hash: hash,
            created_at: now,
            last_used_at: now,
        };
        self.storage.clip_save(&item).await?;
        self.cache_upsert(item);
        self.prune().await;
        self.bump();
        Ok(true)
    }

    // —— 历史读取 / 操作 ——

    /// 全量搜索（覆盖缓存窗口之外的历史）。主视图后台去抖调用，匹配 preview/text
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<ClipItem>> {
        self.storage.clip_search(query, limit).await
    }

    /// 复制条目回剪贴板（不自动粘贴）
    pub async fn copy_to_clipboard(&self, item: &ClipItem) -> Result<()> {
        match item.kind {
            ClipKind::Image => {
                if let Some(png) = self.load_image(item).await? {
                    self.driver.write_image_png(&png)?;
                }
            }
            ClipKind::Files => {
                // 文件失效校验：路径已不存在则拒绝复制，提示用户
                if !self.driver.paths_exist(&item.files) {
                    return Err(DomainError::NotFound("文件已移动或删除".into()));
                }
                self.driver.write_files(&item.files)?;
            }
            _ => {
                if let Some(text) = &item.text {
                    self.driver.write_text(text, item.rtf.as_deref())?;
                }
            }
        }
        // 复制即提升为最新
        let mut latest = item.clone();
        latest.last_used_at = Utc::now();
        self.storage.clip_save(&latest).await?;
        self.cache_upsert(latest);
        self.bump();
        Ok(())
    }

    /// 复制并粘贴到目标应用（需辅助功能权限；无权限降级为仅复制并返回 Err）
    pub async fn paste_to_app(&self, item: &ClipItem, bundle_id: Option<&str>) -> Result<()> {
        self.copy_to_clipboard(item).await?;
        self.driver.paste_to_app(bundle_id)
    }

    /// 仅复制纯文本（剥离 RTF 富文本格式）；非文本类型回退普通复制
    pub async fn copy_as_plain_text(&self, item: &ClipItem) -> Result<()> {
        match &item.text {
            Some(text) => {
                self.driver.write_text(text, None)?;
                let mut latest = item.clone();
                latest.rtf = None;
                latest.last_used_at = Utc::now();
                self.storage.clip_save(&latest).await?;
                self.cache_upsert(latest);
                self.bump();
                Ok(())
            }
            None => self.copy_to_clipboard(item).await,
        }
    }

    /// 来源应用图标 PNG（按 bundle_id 缓存）；卡片右上角显示用
    pub fn app_icon(&self, bundle_id: &str) -> Option<std::sync::Arc<Vec<u8>>> {
        self.driver.app_icon_png(bundle_id)
    }

    /// 读原图明文 PNG（读密文 → 解密）；非图片或无图返回 None
    pub async fn load_image(&self, item: &ClipItem) -> Result<Option<Vec<u8>>> {
        match &item.image_path {
            Some(p) => {
                let enc = self.driver.read_media(p)?;
                Ok(Some(self.storage.unseal(&enc).await?))
            }
            None => Ok(None),
        }
    }

    /// 读缩略图明文 PNG（列表展示用）；无缩略图回退原图
    pub async fn load_thumb(&self, item: &ClipItem) -> Result<Option<Vec<u8>>> {
        match &item.thumb_path {
            Some(p) => {
                let enc = self.driver.read_media(p)?;
                Ok(Some(self.storage.unseal(&enc).await?))
            }
            None => self.load_image(item).await,
        }
    }

    /// 清理孤儿媒体文件：磁盘上有、但库里任何条目都不引用的文件（崩溃/不一致残留）。
    /// 启动时调一次，返回清理数量
    pub async fn cleanup_orphans(&self) -> Result<usize> {
        let items = self.storage.clip_list().await?;
        let mut referenced = std::collections::HashSet::new();
        for it in &items {
            if let Some(p) = &it.image_path {
                referenced.insert(p.clone());
            }
            if let Some(p) = &it.thumb_path {
                referenced.insert(p.clone());
            }
        }
        let mut removed = 0;
        for path in self.driver.list_media()? {
            if !referenced.contains(&path) {
                if let Err(e) = self.driver.remove_media(&path) {
                    warn!(error = %e, path, "remove orphan media failed");
                } else {
                    removed += 1;
                }
            }
        }
        if removed > 0 {
            debug!(removed, "orphan media cleaned");
        }
        Ok(removed)
    }

    /// 用默认浏览器打开链接
    pub fn open_url(&self, url: &str) -> Result<()> {
        self.driver.open_url(url)
    }

    /// 在 Finder 中显示文件
    pub fn reveal_in_finder(&self, paths: &[String]) -> Result<()> {
        self.driver.reveal_in_finder(paths)
    }

    pub async fn delete(&self, item: &ClipItem) -> Result<()> {
        self.storage.clip_delete(&item.id).await?;
        self.cache_remove(&item.id);
        for path in [&item.image_path, &item.thumb_path].into_iter().flatten() {
            self.driver.remove_media(path)?;
        }
        self.bump();
        Ok(())
    }

    pub async fn clear(&self) -> Result<()> {
        let images = self.storage.clip_clear().await?;
        self.cache_clear();
        self.cleanup_media(images);
        self.bump();
        Ok(())
    }

    async fn prune(&self) {
        match self.storage.clip_prune(MAX_ITEMS, MAX_AGE_DAYS).await {
            Ok(images) => self.cleanup_media(images),
            Err(e) => warn!(error = %e, "clip prune failed"),
        }
    }

    fn cleanup_media(&self, paths: Vec<String>) {
        for path in paths {
            if let Err(e) = self.driver.remove_media(&path) {
                warn!(error = %e, path, "remove clip media failed");
            }
        }
    }
}

/// 纯判定：是否记录该次采集（无 IO，便于测试）
pub fn decide_capture(
    captured: &CapturedClip,
    settings: &ClipboardSettings,
    source: Option<&ClipSource>,
) -> CaptureDecision {
    if captured.concealed {
        return CaptureDecision::Skip("concealed");
    }
    if let Some(src) = source
        && settings.blacklist.iter().any(|b| b == &src.bundle_id)
    {
        return CaptureDecision::Skip("blacklisted");
    }

    // 文件优先，其次图片，最后文本（与驱动读取优先级一致）
    if !captured.files.is_empty() {
        let joined = captured.files.join("\n");
        return CaptureDecision::Record {
            hash: hash_hex(joined.as_bytes()),
            kind: ClipKind::Files,
        };
    }
    if let Some(png) = &captured.image_png {
        if (png.len() as u64) > settings.max_item_bytes {
            return CaptureDecision::Skip("image too large");
        }
        if !settings.capture_images {
            return CaptureDecision::Skip("image capture disabled");
        }
        return CaptureDecision::Record {
            hash: hash_hex(png),
            kind: ClipKind::Image,
        };
    }
    if let Some(text) = &captured.text {
        if text.trim().is_empty() {
            return CaptureDecision::Skip("empty text");
        }
        if (text.len() as u64) > settings.max_item_bytes {
            return CaptureDecision::Skip("text too large");
        }
        return CaptureDecision::Record {
            hash: hash_hex(text.as_bytes()),
            kind: classify_text(text),
        };
    }
    CaptureDecision::Skip("empty")
}

fn hash_hex(bytes: &[u8]) -> String {
    format!("{:016x}", fnv1a_hash(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> ClipboardSettings {
        ClipboardSettings::default()
    }

    fn text_clip(s: &str) -> CapturedClip {
        CapturedClip {
            text: Some(s.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn concealed_is_skipped() {
        let c = CapturedClip {
            concealed: true,
            ..Default::default()
        };
        assert_eq!(
            decide_capture(&c, &settings(), None),
            CaptureDecision::Skip("concealed")
        );
    }

    #[test]
    fn blacklist_skips_by_bundle() {
        let mut s = settings();
        s.blacklist.push("com.1password.1password".into());
        let src = ClipSource {
            bundle_id: "com.1password.1password".into(),
            name: "1Password".into(),
        };
        assert_eq!(
            decide_capture(&text_clip("secret"), &s, Some(&src)),
            CaptureDecision::Skip("blacklisted")
        );
    }

    #[test]
    fn empty_and_oversize_text_skipped() {
        assert_eq!(
            decide_capture(&text_clip("   "), &settings(), None),
            CaptureDecision::Skip("empty text")
        );
        let mut s = settings();
        s.max_item_bytes = 4;
        assert_eq!(
            decide_capture(&text_clip("toolong"), &s, None),
            CaptureDecision::Skip("text too large")
        );
    }

    #[test]
    fn text_classified_and_hashed() {
        let d = decide_capture(&text_clip("https://example.com/x"), &settings(), None);
        match d {
            CaptureDecision::Record { kind, hash } => {
                assert_eq!(kind, ClipKind::Link);
                assert_eq!(hash.len(), 16);
            }
            _ => panic!("应记录"),
        }
    }

    #[test]
    fn files_take_priority_over_text() {
        let c = CapturedClip {
            text: Some("/path/as/text".into()),
            files: vec!["/path/a".into(), "/path/b".into()],
            ..Default::default()
        };
        assert!(matches!(
            decide_capture(&c, &settings(), None),
            CaptureDecision::Record {
                kind: ClipKind::Files,
                ..
            }
        ));
    }

    #[test]
    fn image_respects_size_and_toggle() {
        let big = CapturedClip {
            image_png: Some(vec![0u8; 100]),
            ..Default::default()
        };
        let mut s = settings();
        s.max_item_bytes = 50;
        assert_eq!(
            decide_capture(&big, &s, None),
            CaptureDecision::Skip("image too large")
        );

        let small = CapturedClip {
            image_png: Some(vec![0u8; 10]),
            ..Default::default()
        };
        let mut s2 = settings();
        s2.capture_images = false;
        assert_eq!(
            decide_capture(&small, &s2, None),
            CaptureDecision::Skip("image capture disabled")
        );
        assert!(matches!(
            decide_capture(&small, &settings(), None),
            CaptureDecision::Record {
                kind: ClipKind::Image,
                ..
            }
        ));
    }
}
