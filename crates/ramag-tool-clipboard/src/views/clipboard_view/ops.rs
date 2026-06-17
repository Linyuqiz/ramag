//! ClipboardView 异步操作：重载 / 复制 / 删除 / 清空 / 键盘导航

use gpui::{Context, ScrollStrategy, Window};
use gpui_component::notification::Notification;
use ramag_domain::entities::{ClipId, ClipItem, ClipboardSettings};
use tracing::error;

use super::ClipboardView;
use crate::views::helpers::filter_items;

/// 全量搜索去抖：输入停顿此间隔后才触发后台扫描
const SEARCH_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);
/// 全量搜索结果上限
const SEARCH_LIMIT: usize = 500;

impl ClipboardView {
    /// 同步从 service 缓存重载最近窗口快照（无 IO、无解密；旧版异步全表解密已废弃）
    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        self.loaded_revision = self.service.revision();
        self.items = self.service.cached_snapshot();
        // 选中项若已被删除则清空
        if let Some(sel) = &self.selected
            && !self.items.iter().any(|i| &i.id == sel)
        {
            self.selected = None;
        }
        cx.notify();
    }

    pub(super) fn load_settings(&mut self, cx: &mut Context<Self>) {
        let svc = self.service.clone();
        cx.spawn(async move |this, cx| {
            let settings = svc.load_settings().await;
            let _ = this.update(cx, |this, cx| {
                this.settings = settings;
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn save_settings(&mut self, settings: ClipboardSettings, cx: &mut Context<Self>) {
        self.settings = settings.clone();
        let svc = self.service.clone();
        cx.spawn(async move |_this, _cx| {
            if let Err(e) = svc.save_settings(&settings).await {
                error!(error = %e, "save clip settings failed");
            }
        })
        .detach();
        cx.notify();
    }

    /// 当前过滤+排序后的可见条目（clone 出 owned 列表供渲染与键盘导航共用）
    pub(super) fn visible_items(&self, cx: &gpui::App) -> Vec<ClipItem> {
        let query = self.search.read(cx).value().to_string();
        if query.trim().is_empty() {
            return filter_items(&self.items, "", self.filter)
                .into_iter()
                .cloned()
                .collect();
        }
        // 即时层：缓存窗口匹配（输入即显示）；补充层：后台全量结果（去重，缓存优先）
        let mut seen = std::collections::HashSet::new();
        let mut out: Vec<ClipItem> = Vec::new();
        for it in filter_items(&self.items, &query, self.filter) {
            seen.insert(it.id.clone());
            out.push(it.clone());
        }
        for it in &self.search_results {
            if self.filter.is_none_or(|k| it.kind == k) && !seen.contains(&it.id) {
                out.push(it.clone());
            }
        }
        out
    }

    /// 搜索框变化：去抖后台全量搜索，补充缓存窗口之外的匹配
    pub(super) fn schedule_search(&mut self, cx: &mut Context<Self>) {
        self.search_gen = self.search_gen.wrapping_add(1);
        let generation = self.search_gen;
        let query = self.search.read(cx).value().to_string();
        if query.trim().is_empty() {
            self.search_results.clear();
            return;
        }
        let svc = self.service.clone();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            // 去抖：期间又有输入则代号已变，放弃本次
            if this
                .update(cx, |this, _| this.search_gen != generation)
                .unwrap_or(true)
            {
                return;
            }
            let result = svc.search(&query, SEARCH_LIMIT).await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                if this.search_gen == generation {
                    this.search_results = result;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn copy_clip(&mut self, item: ClipItem, cx: &mut Context<Self>) {
        let svc = self.service.clone();
        cx.spawn(async move |this, cx| {
            let result = svc.copy_to_clipboard(&item).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.pending_notification = Some(Notification::info("已复制到剪贴板"))
                    }
                    Err(e) => {
                        error!(error = %e, "copy clip failed");
                        this.pending_notification = Some(Notification::error(e.to_string()));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn copy_plain(&mut self, item: ClipItem, cx: &mut Context<Self>) {
        let svc = self.service.clone();
        cx.spawn(async move |this, cx| {
            let result = svc.copy_as_plain_text(&item).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.pending_notification = Some(Notification::info("已复制为纯文本"))
                    }
                    Err(e) => {
                        error!(error = %e, "copy plain failed");
                        this.pending_notification = Some(Notification::error(e.to_string()));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 浏览器打开链接（同步调用，失败弹通知）
    pub(super) fn open_link(&mut self, url: String, cx: &mut Context<Self>) {
        if let Err(e) = self.service.open_url(&url) {
            error!(error = %e, "open url failed");
            self.pending_notification = Some(Notification::error(e.to_string()));
            cx.notify();
        }
    }

    /// Finder 中显示文件
    pub(super) fn reveal_files(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        if let Err(e) = self.service.reveal_in_finder(&paths) {
            error!(error = %e, "reveal in finder failed");
            self.pending_notification = Some(Notification::error(e.to_string()));
            cx.notify();
        }
    }

    pub(super) fn delete_clip(&mut self, item: ClipItem, cx: &mut Context<Self>) {
        let svc = self.service.clone();
        cx.spawn(async move |this, cx| {
            if let Err(e) = svc.delete(&item).await {
                error!(error = %e, "delete clip failed");
            }
            let _ = this.update(cx, |this, cx| this.reload(cx));
        })
        .detach();
    }

    pub(super) fn clear_all(&mut self, cx: &mut Context<Self>) {
        let svc = self.service.clone();
        cx.spawn(async move |this, cx| {
            if let Err(e) = svc.clear().await {
                error!(error = %e, "clear clips failed");
            }
            let _ = this.update(cx, |this, cx| this.reload(cx));
        })
        .detach();
    }

    pub(super) fn select_id(&mut self, id: ClipId, cx: &mut Context<Self>) {
        self.selected = Some(id);
        // 选中条目即回到详情视图，关闭设置面板
        self.show_settings = false;
        cx.notify();
    }

    /// 键盘上/下移动选中（基于可见列表）
    pub(super) fn move_selection(&mut self, delta: i32, cx: &mut Context<Self>) {
        let visible = self.visible_items(cx);
        if visible.is_empty() {
            return;
        }
        let cur = self
            .selected
            .as_ref()
            .and_then(|sel| visible.iter().position(|i| &i.id == sel));
        let next = match cur {
            Some(idx) => (idx as i32 + delta).clamp(0, visible.len() as i32 - 1) as usize,
            None => {
                if delta > 0 {
                    0
                } else {
                    visible.len() - 1
                }
            }
        };
        self.selected = Some(visible[next].id.clone());
        self.list_scroll.scroll_to_item(next, ScrollStrategy::Top);
        cx.notify();
    }

    /// 复制当前选中项（快捷键入口）
    pub(super) fn copy_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(item) = self.selected_item(cx) {
            self.copy_clip(item, cx);
        }
    }

    pub(super) fn delete_selected(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = self.selected_item(cx) {
            self.delete_clip(item, cx);
        }
    }

    pub(super) fn selected_item(&self, _cx: &gpui::App) -> Option<ClipItem> {
        let sel = self.selected.as_ref()?;
        self.items.iter().find(|i| &i.id == sel).cloned()
    }

    /// 取图片的解密内存图片（thumb=true 用缩略图，否则原图）。
    /// 缓存命中同步返回；miss 异步解密填充后 notify，本帧返回 None（占位）
    pub(super) fn image_for(
        &self,
        item: &ClipItem,
        thumb: bool,
        cx: &mut Context<Self>,
    ) -> Option<std::sync::Arc<gpui::Image>> {
        let path = if thumb {
            item.thumb_path
                .clone()
                .or_else(|| item.image_path.clone())?
        } else {
            item.image_path.clone()?
        };
        if let Some(img) = self.img_cache.peek(&path) {
            return Some(img);
        }
        if self.img_cache.begin_load(&path) {
            let svc = self.service.clone();
            let item = item.clone();
            cx.spawn(async move |this, cx| {
                let loaded = if thumb {
                    svc.load_thumb(&item).await
                } else {
                    svc.load_image(&item).await
                };
                let _ = this.update(cx, |this, cx| match loaded {
                    Ok(Some(bytes)) => {
                        let image = std::sync::Arc::new(gpui::Image::from_bytes(
                            gpui::ImageFormat::Png,
                            bytes,
                        ));
                        this.img_cache.insert(path, image);
                        cx.notify();
                    }
                    _ => this.img_cache.fail(&path),
                });
            })
            .detach();
        }
        None
    }
}
