//! ClipboardView：剪贴板历史主视图。左卡片流 + 右详情，搜索 / 类型筛选 / 钉住。
//! 历史由 App 级采集循环写入 storage；视图轮询 `service.revision()` 仅在变化时重载

mod card;
mod detail;
mod ops;
mod render;
mod settings;

use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Subscription,
    UniformListScrollHandle, Window,
};
use gpui_component::input::{InputEvent, InputState};
use ramag_app::ClipboardService;
use ramag_domain::entities::{ClipId, ClipItem, ClipKind, ClipboardSettings};

/// 轮询间隔：采集循环写库后，视图最迟此间隔内刷新
const POLL_INTERVAL: Duration = Duration::from_millis(600);

/// 视图事件（预留：未来与悬浮抽屉联动）
#[derive(Debug, Clone)]
pub enum ClipboardEvent {
    /// 条目已复制（可用于 toast）
    Copied,
}

pub struct ClipboardView {
    pub(super) service: Arc<ClipboardService>,
    /// 全量历史（storage 已按 last_used_at desc）
    pub(super) items: Vec<ClipItem>,
    pub(super) settings: ClipboardSettings,
    pub(super) search: Entity<InputState>,
    /// 类型筛选；None = 全部
    pub(super) filter: Option<ClipKind>,
    pub(super) selected: Option<ClipId>,
    /// 上次已加载的版本号，轮询时与 service.revision() 比对
    pub(super) loaded_revision: u64,
    /// 设置面板是否展开
    pub(super) show_settings: bool,
    pub(super) list_scroll: UniformListScrollHandle,
    pub(super) focus_handle: FocusHandle,
    pub(super) pending_notification: Option<gpui_component::notification::Notification>,
    /// 图片解密缓存（缩略图 / 原图）
    pub(super) img_cache: crate::views::image_cache::ImageCache,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ClipboardEvent> for ClipboardView {}

impl Focusable for ClipboardView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ClipboardView {
    pub fn new(
        service: Arc<ClipboardService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索剪贴历史…"));

        let mut subscriptions = Vec::new();
        // 搜索框输入即重渲染（过滤是纯内存操作）
        subscriptions.push(
            cx.subscribe(&search, |_this: &mut Self, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    cx.notify();
                }
            }),
        );

        let mut view = Self {
            service,
            items: Vec::new(),
            settings: ClipboardSettings::default(),
            search,
            filter: None,
            selected: None,
            loaded_revision: 0,
            show_settings: false,
            list_scroll: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            pending_notification: None,
            img_cache: crate::views::image_cache::ImageCache::new(),
            _subscriptions: subscriptions,
        };
        view.load_settings(cx);
        view.reload(cx);
        view.start_polling(cx);
        view
    }

    /// 后台计时轮询：版本号变化才重载（重载会全表解密，故不每拍执行）
    fn start_polling(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                let alive = this
                    .update(cx, |this, cx| {
                        if this.service.revision() != this.loaded_revision {
                            this.reload(cx);
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
    }
}
