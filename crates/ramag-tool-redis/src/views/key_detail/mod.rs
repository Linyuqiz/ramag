//! Key 详情：按 RedisValue variant dispatch 到 scalar / list_block / hash_block / set_block / zset_block / stream_block

mod hash_block;
mod header;
mod helpers;
mod list_block;
mod scalar;
mod set_block;
mod stream_block;
mod zset_block;

use std::sync::Arc;

use gpui::{
    Context, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render, Styled,
    Window, div, prelude::*, px,
};
use gpui_component::{ActiveTheme, scroll::ScrollableElement as _, v_flex};
use ramag_app::RedisService;
use ramag_domain::entities::{ConnectionConfig, RedisValue};
use tracing::{error, info};

use helpers::{futures_join, render_value};

use crate::views::value_display::ViewMode;

#[derive(Debug, Clone)]
pub enum KeyDetailEvent {
    /// 详情面板触发 DEL 后通知 KeyTree 刷新
    Deleted(String),
    /// 请求编辑 TTL，弹窗由上层 Session 处理。(key, 当前 ttl_ms)
    RequestEditTtl(String, Option<i64>),
    /// 请求编辑 String 值（弹窗由上层 Session 处理）
    /// (key 名, 当前文本值)
    RequestEditValue(String, String),
    /// 请求新增 Hash 字段（弹窗）
    /// (key 名)
    RequestAddHashField(String),
    /// 请求编辑 Hash 字段（弹窗）
    /// (key 名, field, 当前 value 文本预览)
    RequestEditHashField(String, String, String),
    /// 请求新增 List 元素（弹窗）
    /// (key 名)
    RequestAddListElement(String),
    /// 请求新增 Set 元素（弹窗）
    /// (key 名)
    RequestAddSetElement(String),
    /// 请求新增 ZSet 元素（弹窗）
    /// (key 名)
    RequestAddZSetElement(String),
    /// 请求编辑 ZSet 成员的 score（弹窗）
    /// (key 名, member, 当前 score 字符串)
    RequestEditZSetScore(String, String, String),
    /// 请求新增 Stream 条目（弹窗）
    /// (key 名)
    RequestAddStreamEntry(String),
    /// 请求删除 Key（由上层 Session 弹二次确认）
    /// (key 名)
    RequestDeleteKey(String),
    /// 请求删除 Hash 字段（由上层 Session 弹二次确认）
    /// (key 名, field 名)
    RequestDeleteHashField(String, String),
    /// 请求删除 List 元素（由上层 Session 弹二次确认）
    /// (key 名, 元素值, 序号)
    RequestDeleteListElement(String, String, usize),
    /// 请求删除 Set 成员（由上层 Session 弹二次确认）
    /// (key 名, 成员值)
    RequestDeleteSetElement(String, String),
    /// 请求删除 ZSet 成员（由上层 Session 弹二次确认）
    /// (key 名, 成员值)
    RequestDeleteZSetMember(String, String),
    /// 请求删除 Stream 条目（由上层 Session 弹二次确认）
    /// (key 名, entry_id)
    RequestDeleteStreamEntry(String, String),
}

pub struct KeyDetailPanel {
    service: Arc<RedisService>,
    config: Option<ConnectionConfig>,
    /// pub(super) 让 header 模块读 db / value / ttl_ms / 大小估算状态
    pub(super) db: u8,
    /// 当前 key 名（None = 未选中任何 key）
    key: Option<String>,
    /// 当前 key 的值（fetch 后填充）
    pub(super) value: Option<RedisValue>,
    /// TTL 毫秒（-1 永久 / -2 不存在 / >=0 剩余）
    pub(super) ttl_ms: Option<i64>,
    /// 加载状态
    loading: bool,
    error: Option<String>,
    /// 单 Key 字节估算（MEMORY USAGE，需用户主动点击触发）
    pub(super) key_size_bytes: Option<u64>,
    pub(super) estimating_size: bool,
    /// 标量值视图模式：None=按内容自动（JSON 美化 / Raw），Some=用户手动选定
    value_view_mode: Option<ViewMode>,
    /// Session 调 focus_panel 聚焦后，cmd-w 等 action 走焦点链路由到 Session
    focus_handle: FocusHandle,
}

impl EventEmitter<KeyDetailEvent> for KeyDetailPanel {}

impl Focusable for KeyDetailPanel {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl KeyDetailPanel {
    pub fn new(service: Arc<RedisService>, cx: &mut Context<Self>) -> Self {
        Self {
            service,
            config: None,
            db: 0,
            key: None,
            value: None,
            ttl_ms: None,
            loading: false,
            error: None,
            key_size_bytes: None,
            value_view_mode: None,
            focus_handle: cx.focus_handle(),
            estimating_size: false,
        }
    }

    pub fn set_connection(
        &mut self,
        config: Option<ConnectionConfig>,
        db: u8,
        cx: &mut Context<Self>,
    ) {
        self.config = config;
        self.db = db;
        self.key = None;
        self.value = None;
        self.ttl_ms = None;
        self.error = None;
        self.key_size_bytes = None;
        self.value_view_mode = None;
        cx.notify();
    }

    /// 加载某 key 的值（由 Session 在收到 KeyTreeEvent::Selected 时调用）
    pub fn load_key(&mut self, key: String, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        self.key = Some(key.clone());
        self.value = None;
        self.ttl_ms = None;
        self.loading = true;
        self.error = None;
        self.key_size_bytes = None;
        self.value_view_mode = None;
        cx.notify();

        let svc = self.service.clone();
        let db = self.db;
        cx.spawn(async move |this, cx| {
            // 并发拉值 + TTL
            let (value_res, ttl_res) = futures_join(
                svc.get_value(&config, db, &key),
                svc.key_ttl(&config, db, &key),
            )
            .await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match value_res {
                    Ok(v) => this.value = Some(v),
                    Err(e) => {
                        error!(error = %e, "load key value failed");
                        this.error = Some(format!("加载值失败：{e}"));
                    }
                }
                this.ttl_ms = ttl_res.ok();
                cx.notify();
            });
        })
        .detach();
    }

    /// 删除 Hash 中的某个字段（HDEL key field）→ 完成后重载详情
    pub fn delete_hash_field(&mut self, field: String, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        let Some(key) = self.key.clone() else {
            return;
        };
        let svc = self.service.clone();
        let db = self.db;
        let key_for_reload = key.clone();
        let argv = vec!["HDEL".to_string(), key, field.clone()];
        cx.spawn(async move |this, cx| {
            let result = svc.execute_command(&config, db, argv).await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(_) => {
                    info!(?field, "hash field deleted");
                    this.load_key(key_for_reload, cx);
                }
                Err(e) => {
                    error!(error = %e, "delete hash field failed");
                    this.error = Some(format!("删除字段失败：{e}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 公开重载方法（外部如 Session 在弹窗保存后调用）
    pub fn reload_current(&mut self, cx: &mut Context<Self>) {
        if let Some(k) = self.key.clone() {
            self.load_key(k, cx);
        }
    }

    /// 当前正在展示的 key 名（None = 未选中任何 key）
    pub fn current_key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    /// 清空当前展示（恢复"未选中 Key"占位态）
    pub fn clear_key(&mut self, cx: &mut Context<Self>) {
        self.key = None;
        self.value = None;
        self.ttl_ms = None;
        self.loading = false;
        self.error = None;
        self.key_size_bytes = None;
        self.estimating_size = false;
        self.value_view_mode = None;
        cx.notify();
    }

    /// 把整面板焦点拿到（Session 在初始化 / 选中 key 时调）
    pub fn focus_panel(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// scalar 视图模式切换按钮调用：固定为用户选择的模式（覆盖按内容自动选择）
    pub(super) fn set_value_view_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        if self.value_view_mode != Some(mode) {
            self.value_view_mode = Some(mode);
            cx.notify();
        }
    }

    /// `MEMORY USAGE` 写入 self.key_size_bytes，由 header 估算按钮调用
    pub(super) fn estimate_size(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        let Some(key) = self.key.clone() else {
            return;
        };
        self.estimating_size = true;
        cx.notify();

        let svc = self.service.clone();
        let db = self.db;
        cx.spawn(async move |this, cx| {
            let argv = vec!["MEMORY".into(), "USAGE".into(), key.clone()];
            let result = svc.execute_command(&config, db, argv).await;
            let _ = this.update(cx, |this, cx| {
                this.estimating_size = false;
                match result {
                    Ok(RedisValue::Int(n)) if n >= 0 => {
                        this.key_size_bytes = Some(n as u64);
                        info!(?key, n, "memory usage ok");
                    }
                    Ok(RedisValue::Nil) => {
                        this.key_size_bytes = None;
                        info!(?key, "memory usage nil (key gone)");
                    }
                    Ok(other) => {
                        error!(?other, "memory usage unexpected response");
                        this.error = Some("MEMORY USAGE 应答异常（可能服务端不支持）".to_string());
                    }
                    Err(e) => {
                        error!(error = %e, "memory usage failed");
                        this.error = Some(format!("MEMORY USAGE 失败：{e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 通用容器单元素删除：发命令成功后重载详情
    fn delete_element(
        &mut self,
        argv: Vec<String>,
        log_label: &'static str,
        cx: &mut Context<Self>,
    ) {
        let Some(config) = self.config.clone() else {
            return;
        };
        let Some(key) = self.key.clone() else {
            return;
        };
        let svc = self.service.clone();
        let db = self.db;
        cx.spawn(async move |this, cx| {
            let result = svc.execute_command(&config, db, argv).await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(_) => {
                    info!(label = log_label, "element deleted");
                    this.load_key(key, cx);
                }
                Err(e) => {
                    error!(error = %e, label = log_label, "delete element failed");
                    this.error = Some(format!("删除元素失败：{e}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// 删除 List 中的某个值（LREM key 1 value）
    pub fn delete_list_element(&mut self, value: String, cx: &mut Context<Self>) {
        let key = match &self.key {
            Some(k) => k.clone(),
            None => return,
        };
        self.delete_element(vec!["LREM".into(), key, "1".into(), value], "lrem", cx);
    }

    /// 删除 Set 中的某个成员（SREM key member）
    pub fn delete_set_element(&mut self, member: String, cx: &mut Context<Self>) {
        let key = match &self.key {
            Some(k) => k.clone(),
            None => return,
        };
        self.delete_element(vec!["SREM".into(), key, member], "srem", cx);
    }

    /// 删除 Stream 中的某条 entry（XDEL key entry_id）
    pub fn delete_stream_entry(&mut self, entry_id: String, cx: &mut Context<Self>) {
        let key = match &self.key {
            Some(k) => k.clone(),
            None => return,
        };
        self.delete_element(vec!["XDEL".into(), key, entry_id], "xdel", cx);
    }

    /// 删除 ZSet 中的某个成员（ZREM key member）
    pub fn delete_zset_member(&mut self, member: String, cx: &mut Context<Self>) {
        let key = match &self.key {
            Some(k) => k.clone(),
            None => return,
        };
        self.delete_element(vec!["ZREM".into(), key, member], "zrem", cx);
    }

    /// 真正发 DEL 命令删除当前 key（由上层 Session 在二次确认通过后调用）
    pub fn delete_key_now(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        let Some(key) = self.key.clone() else {
            return;
        };
        let svc = self.service.clone();
        let db = self.db;
        cx.spawn(async move |this, cx| {
            let result = svc.delete_key(&config, db, &key).await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(_) => {
                    info!(?key, "key deleted");
                    let removed_key = key.clone();
                    this.key = None;
                    this.value = None;
                    this.ttl_ms = None;
                    cx.emit(KeyDetailEvent::Deleted(removed_key));
                    cx.notify();
                }
                Err(e) => {
                    error!(error = %e, "delete key failed");
                    this.error = Some(format!("删除失败：{e}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl Render for KeyDetailPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let border = theme.border;
        let bg = theme.background;
        let accent = theme.accent;

        let Some(key) = self.key.clone() else {
            return v_flex()
                .size_full()
                .bg(bg)
                .track_focus(&self.focus_handle)
                .items_center()
                .justify_center()
                .gap(px(6.0))
                .child(
                    div()
                        .text_sm()
                        .text_color(muted_fg)
                        .child("从左侧选择一个 Key 查看详情"),
                )
                .into_any_element();
        };

        let header = header::render_header(self, &key, fg, muted_fg, accent, border, cx);
        let view_mode = self.value_view_mode;

        let body: gpui::AnyElement = if self.loading {
            div()
                .py(px(28.0))
                .text_center()
                .text_sm()
                .text_color(muted_fg)
                .child("加载中...")
                .into_any_element()
        } else if let Some(err) = self.error.clone() {
            div()
                .p(px(14.0))
                .text_sm()
                .text_color(gpui::red())
                .child(err)
                .into_any_element()
        } else if let Some(v) = self.value.clone() {
            // 标量 String/Bytes 走 scalar 模块（含 Gzip 提示 + 编辑按钮）
            // 其他容器类型走 helpers::render_value 分发
            match &v {
                RedisValue::Text(_) | RedisValue::Bytes(_) => {
                    scalar::render_scalar(&key, &v, view_mode, fg, muted_fg, border, cx, window)
                        .into_any_element()
                }
                _ => render_value(&v, &key, cx, fg, muted_fg, accent, border),
            }
        } else {
            div()
                .p(px(14.0))
                .text_sm()
                .text_color(muted_fg)
                .child("(无值)")
                .into_any_element()
        };

        v_flex()
            .size_full()
            .bg(bg)
            .track_focus(&self.focus_handle)
            .child(header)
            // 滚动区分两层：外层 flex_1 + min_h_0 给出「减去 header 后的确定高度」，
            // 内层 overflow_y_scrollbar 在该确定高度内滚动。
            // 不能直接 .flex_1().min_h_0().overflow_y_scrollbar()——该包装器只继承 size 样式且会给
            // 内容元素重新加 flex_1，min_h_0 落到内容上会把它压扁到视口高度，从而永不溢出、无法滚动
            .child(
                div().flex_1().min_h_0().child(
                    div()
                        .size_full()
                        .overflow_y_scrollbar()
                        .child(div().w_full().p(px(14.0)).child(body)),
                ),
            )
            .into_any_element()
    }
}
