//! 底部悬浮抽屉：全局热键唤起，仿 Paste.app 横向大卡片墙。
//! 双击卡片 / 数字键 / 回车 → 写回剪贴板并粘贴回原应用。
//! 由 ramag-bin 在 PopUp（NonactivatingPanel）窗口内装载

mod card;

use std::sync::Arc;

use gpui::{
    Context, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent, ParentElement, Render,
    ScrollHandle, Styled, Subscription, Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Sizable as _, h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use ramag_app::ClipboardService;
use ramag_domain::entities::ClipItem;

use crate::views::helpers::filter_items;

/// 过滤后最多展示的条目数（搜索在全量历史上进行，仅显示前 N 张）
const DRAWER_LIMIT: usize = 60;

pub struct ClipboardDrawer {
    service: Arc<ClipboardService>,
    /// 全量历史（搜索在此过滤）
    items: Vec<ClipItem>,
    /// 过滤后可见列表上的选中下标
    selected: usize,
    search: Entity<InputState>,
    /// 唤起时记录的前台应用 bundle id，粘贴时激活回去
    target_bundle: Option<String>,
    auto_paste: bool,
    scroll: ScrollHandle,
    focus_handle: FocusHandle,
    pub(super) img_cache: crate::views::image_cache::ImageCache,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for ClipboardDrawer {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ClipboardDrawer {
    pub fn new(
        service: Arc<ClipboardService>,
        target_bundle: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索…"));

        let mut subs = Vec::new();
        // 输入即过滤：内容变化重置选中到首项；回车粘贴当前选中
        subs.push(cx.subscribe_in(
            &search,
            window,
            |this: &mut Self, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    this.selected = 0;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.paste(this.selected, window, cx),
                _ => {}
            },
        ));
        // 搜索框默认聚焦，唤起即可打字过滤
        search.update(cx, |s, cx| s.focus(window, cx));

        let view = Self {
            service: service.clone(),
            items: Vec::new(),
            selected: 0,
            search,
            target_bundle,
            auto_paste: true,
            scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            img_cache: crate::views::image_cache::ImageCache::new(),
            _subscriptions: subs,
        };
        cx.spawn(async move |this, cx| {
            let settings = service.load_settings().await;
            let items = service.list().await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.items = items;
                this.auto_paste = settings.auto_paste;
                cx.notify();
            });
        })
        .detach();
        view
    }

    pub(super) fn service(&self) -> &Arc<ClipboardService> {
        &self.service
    }

    /// 取缩略图解密内存图片；缓存命中同步返回，miss 异步解密填充后 notify
    pub(super) fn thumb_image(
        &self,
        item: &ClipItem,
        cx: &mut Context<Self>,
    ) -> Option<std::sync::Arc<gpui::Image>> {
        let path = item
            .thumb_path
            .clone()
            .or_else(|| item.image_path.clone())?;
        if let Some(img) = self.img_cache.peek(&path) {
            return Some(img);
        }
        if self.img_cache.begin_load(&path) {
            let svc = self.service.clone();
            let item = item.clone();
            cx.spawn(async move |this, cx| {
                let loaded = svc.load_thumb(&item).await;
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

    /// 按搜索框内容过滤 + 截断的可见列表（渲染 / 选中 / 粘贴共用同一份）
    pub(super) fn visible_items(&self, cx: &gpui::App) -> Vec<ClipItem> {
        let q = self.search.read(cx).value().to_string();
        filter_items(&self.items, &q, None)
            .into_iter()
            .take(DRAWER_LIMIT)
            .cloned()
            .collect()
    }

    /// 粘贴可见列表第 idx 条：写回剪贴板 +（auto_paste 时）激活原应用并模拟 cmd-V，随后关窗
    pub(super) fn paste(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.visible_items(cx).get(idx).cloned() else {
            return;
        };
        let svc = self.service.clone();
        let target = self.target_bundle.clone();
        let auto = self.auto_paste;
        cx.spawn(async move |_, _| {
            let result = if auto {
                svc.paste_to_app(&item, target.as_deref()).await
            } else {
                svc.copy_to_clipboard(&item).await
            };
            if let Err(e) = result {
                tracing::warn!(error = %e, "drawer paste failed");
            }
        })
        .detach();
        window.remove_window();
    }

    /// 键盘：Esc 关闭，cmd-1..9 直贴第 N 张（裸数字 / 方向键留给搜索框，回车由搜索框 PressEnter 处理）
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = ev.keystroke.key.as_str();
        if key == "escape" {
            window.remove_window();
            return;
        }
        if ev.keystroke.modifiers.platform
            && key.len() == 1
            && key.chars().all(|c| ('1'..='9').contains(&c))
        {
            let idx = key.parse::<usize>().unwrap_or(1) - 1;
            self.paste(idx, window, cx);
        }
    }

    /// 顶部工具栏：仿 Paste 居中搜索框
    fn render_topbar(&self) -> impl IntoElement {
        h_flex()
            .w_full()
            .flex_none()
            .h(px(44.0))
            .items_center()
            .justify_center()
            .px(px(16.0))
            .child(
                div()
                    .w(px(360.0))
                    .max_w_full()
                    .child(Input::new(&self.search).small()),
            )
    }
}

impl Render for ClipboardDrawer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 先取 owned 颜色释放 theme 借用，否则与下方 render_card 的 &mut cx 冲突
        let bg = cx.theme().background;
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let focus = self.focus_handle.clone();

        let visible = self.visible_items(cx);
        // 过滤后列表变短时把选中夹回范围内
        if self.selected >= visible.len() {
            self.selected = visible.len().saturating_sub(1);
        }
        let empty = visible.is_empty();

        let topbar = self.render_topbar().into_any_element();
        // for 循环（非 map 闭包）：render_card 需 &mut Context，闭包会触发借用逃逸
        let mut cards = Vec::with_capacity(visible.len());
        for (ix, item) in visible.iter().enumerate() {
            cards.push(self.render_card(ix, item, cx).into_any_element());
        }

        v_flex()
            .key_context("ClipboardDrawer")
            .track_focus(&focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                this.on_key(ev, window, cx);
            }))
            .size_full()
            .bg(bg)
            .border_t_1()
            .border_color(border)
            .child(topbar)
            .child(
                h_flex()
                    .id("drawer-strip")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .gap(px(12.0))
                    .px(px(16.0))
                    .pb(px(12.0))
                    .overflow_x_scroll()
                    .track_scroll(&self.scroll)
                    .when(empty, |this| {
                        this.child(
                            div()
                                .flex_1()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_sm()
                                .text_color(muted)
                                .child("暂无剪贴历史"),
                        )
                    })
                    .children(cards),
            )
    }
}
