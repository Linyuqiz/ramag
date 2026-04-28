//! Key 详情面板：选中 key 后右侧渲染
//!
//! 按 [`RedisValue`] variant dispatch 渲染：
//! - String → 单行/多行文本
//! - List → 序号 + 值
//! - Hash → field-value 双列
//! - Set → 列表（无序）
//! - ZSet → member + score 双列
//! - Stream → entry id + 字段对
//!
//! Stage 15 read-only；Stage 16 加单元格编辑

use std::sync::Arc;

use gpui::{
    ClickEvent, Context, EventEmitter, IntoElement, ParentElement, Render, SharedString, Styled,
    Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement as _,
    v_flex,
};
use ramag_app::RedisService;
use ramag_domain::entities::{ConnectionConfig, RedisValue, StreamEntry};
use tracing::{error, info};

use crate::views::value_display::{self, ViewMode};

#[derive(Debug, Clone)]
pub enum KeyDetailEvent {
    /// key 已删除（详情面板请求 KeyTree 刷新）
    Deleted(String),
    /// 请求编辑 TTL（弹窗由上层 Session 处理）
    /// (key 名, 当前 ttl_ms)
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
}

pub struct KeyDetailPanel {
    service: Arc<RedisService>,
    config: Option<ConnectionConfig>,
    db: u8,
    /// 当前 key 名（None = 未选中任何 key）
    key: Option<String>,
    /// 当前 key 的值（fetch 后填充）
    value: Option<RedisValue>,
    /// TTL 毫秒（-1 永久 / -2 不存在 / >=0 剩余）
    ttl_ms: Option<i64>,
    /// 加载状态
    loading: bool,
    error: Option<String>,
    /// String / Bytes 标量值的展示模式（Raw/JSON/Hex/base64）
    view_mode: ViewMode,
    /// 单 Key 字节估算（MEMORY USAGE，需用户主动点击触发）
    key_size_bytes: Option<u64>,
    estimating_size: bool,
}

impl EventEmitter<KeyDetailEvent> for KeyDetailPanel {}

impl KeyDetailPanel {
    pub fn new(service: Arc<RedisService>) -> Self {
        Self {
            service,
            config: None,
            db: 0,
            key: None,
            value: None,
            ttl_ms: None,
            loading: false,
            error: None,
            view_mode: ViewMode::default(),
            key_size_bytes: None,
            estimating_size: false,
        }
    }

    fn set_view_mode(&mut self, m: ViewMode, cx: &mut Context<Self>) {
        if self.view_mode != m {
            self.view_mode = m;
            cx.notify();
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

    /// 估算当前 Key 占用字节（MEMORY USAGE）→ 写入 self.key_size_bytes
    fn estimate_size(&mut self, cx: &mut Context<Self>) {
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
    /// 注：按值删 1 个；同值有多个时仅删第一个匹配
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

    fn handle_delete(&mut self, cx: &mut Context<Self>) {
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let border = theme.border;
        let bg = theme.background;

        let Some(key) = self.key.clone() else {
            return v_flex()
                .size_full()
                .bg(bg)
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

        let ttl_label = match self.ttl_ms {
            Some(-1) => "永久".to_string(),
            Some(-2) => "已过期".to_string(),
            Some(ms) if ms >= 0 => format_ttl_ms(ms),
            _ => "—".to_string(),
        };
        let accent = theme.accent;
        let key_for_ttl = key.clone();
        let ttl_ms_for_event = self.ttl_ms;

        // 顶部 header：key 名 + 类型 + TTL（可点击编辑） + 删除按钮
        let header = h_flex()
            .w_full()
            .px(px(14.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(border)
            .gap(px(12.0))
            .items_center()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(fg)
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(key.clone()),
                    )
                    .child(
                        h_flex()
                            .gap(px(10.0))
                            .text_xs()
                            .text_color(muted_fg)
                            .child(div().child(format!("DB {}", self.db)))
                            // TTL 行：accent 颜色 + 可点击 → emit RequestEditTtl
                            .child(
                                div()
                                    .id("ttl-edit-trigger")
                                    .text_color(accent)
                                    .cursor_pointer()
                                    .hover(|this| this.opacity(0.75))
                                    .child(format!("TTL {ttl_label} ✎"))
                                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                        cx.emit(KeyDetailEvent::RequestEditTtl(
                                            key_for_ttl.clone(),
                                            ttl_ms_for_event,
                                        ));
                                    })),
                            )
                            .when_some(self.value.as_ref().and_then(|v| v.len()), |this, n| {
                                this.child(div().child(format!("{n} 元素")))
                            })
                            // MEMORY USAGE 入口：未估算时显示按钮；已估算时显示字节数
                            .child(render_size_chip(
                                self.key_size_bytes,
                                self.estimating_size,
                                muted_fg,
                                accent,
                                cx,
                            )),
                    ),
            )
            // [+ X] 按容器类型：Hash → 字段；List/Set/ZSet → 元素
            .when(matches!(self.value, Some(RedisValue::Hash(_))), |this| {
                let key_for_emit = key.clone();
                this.child(
                    Button::new("redis-hash-add-field")
                        .outline()
                        .small()
                        .label("+ 字段")
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(KeyDetailEvent::RequestAddHashField(key_for_emit.clone()));
                        })),
                )
            })
            .when(matches!(self.value, Some(RedisValue::List(_))), |this| {
                let key_for_emit = key.clone();
                this.child(
                    Button::new("redis-list-add-elem")
                        .outline()
                        .small()
                        .label("+ 元素")
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(KeyDetailEvent::RequestAddListElement(key_for_emit.clone()));
                        })),
                )
            })
            .when(matches!(self.value, Some(RedisValue::Set(_))), |this| {
                let key_for_emit = key.clone();
                this.child(
                    Button::new("redis-set-add-elem")
                        .outline()
                        .small()
                        .label("+ 元素")
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(KeyDetailEvent::RequestAddSetElement(key_for_emit.clone()));
                        })),
                )
            })
            .when(matches!(self.value, Some(RedisValue::ZSet(_))), |this| {
                let key_for_emit = key.clone();
                this.child(
                    Button::new("redis-zset-add-elem")
                        .outline()
                        .small()
                        .label("+ 元素")
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(KeyDetailEvent::RequestAddZSetElement(key_for_emit.clone()));
                        })),
                )
            })
            .when(matches!(self.value, Some(RedisValue::Stream(_))), |this| {
                let key_for_emit = key.clone();
                this.child(
                    Button::new("redis-stream-add-entry")
                        .outline()
                        .small()
                        .label("+ 条目")
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(KeyDetailEvent::RequestAddStreamEntry(key_for_emit.clone()));
                        })),
                )
            })
            .child(
                Button::new("redis-key-delete")
                    .danger()
                    .small()
                    .label("删除")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.handle_delete(cx))),
            );

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
            // String / Bytes 标量走"工具条 + 切换视图"渲染
            // 其他容器类型走 render_value 直接 dispatch
            match &v {
                RedisValue::Text(_) | RedisValue::Bytes(_) => self
                    .render_scalar_with_tabs(&key, &v, fg, muted_fg, accent, border, cx)
                    .into_any_element(),
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
            .child(header)
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(div().w_full().p(px(14.0)).child(body)),
            )
            .into_any_element()
    }
}

/// 按 RedisValue variant 分发渲染
///
/// Hash / List / Set / ZSet 走方法版（带 cx + key 用于 emit 编辑/删除事件）；
/// 其他类型走只读 free function
#[allow(clippy::too_many_arguments)]
fn render_value(
    v: &RedisValue,
    key: &str,
    cx: &mut Context<KeyDetailPanel>,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    accent: gpui::Hsla,
    border: gpui::Hsla,
) -> gpui::AnyElement {
    match v {
        RedisValue::Nil => simple_label("(nil)", muted_fg).into_any_element(),
        RedisValue::Text(s) => string_block(s, fg, border).into_any_element(),
        RedisValue::Bytes(b) => bytes_block(b, fg, muted_fg, border).into_any_element(),
        RedisValue::Int(i) => simple_label(&format!("{i} (integer)"), fg).into_any_element(),
        RedisValue::Float(f) => simple_label(&format!("{f} (double)"), fg).into_any_element(),
        RedisValue::Bool(b) => simple_label(&format!("{b} (bool)"), fg).into_any_element(),
        RedisValue::List(items) => {
            render_list_block(cx, items, fg, muted_fg, accent, border).into_any_element()
        }
        RedisValue::Hash(pairs) => {
            render_hash_block(cx, key.to_string(), pairs, fg, muted_fg, accent, border)
                .into_any_element()
        }
        RedisValue::Set(items) => {
            render_set_block(cx, items, fg, muted_fg, accent, border).into_any_element()
        }
        RedisValue::ZSet(pairs) => {
            render_zset_block(cx, key.to_string(), pairs, fg, muted_fg, accent, border)
                .into_any_element()
        }
        RedisValue::Stream(entries) => {
            render_stream_block(cx, entries, fg, muted_fg, accent, border).into_any_element()
        }
        // Array 暂仍走只读（命令应答的兜底，不直接来自 key value）
        RedisValue::Array(items) => list_block(items, fg, muted_fg, border).into_any_element(),
    }
}

fn simple_label(s: &str, color: gpui::Hsla) -> impl IntoElement {
    div()
        .p(px(8.0))
        .text_sm()
        .text_color(color)
        .child(s.to_string())
}

fn string_block(s: &str, fg: gpui::Hsla, border: gpui::Hsla) -> impl IntoElement {
    div()
        .w_full()
        .p(px(10.0))
        .border_1()
        .border_color(border)
        .rounded(px(4.0))
        .text_sm()
        .text_color(fg)
        .font_family("monospace")
        .child(s.to_string())
}

fn bytes_block(
    b: &[u8],
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    border: gpui::Hsla,
) -> impl IntoElement {
    let preview = b
        .iter()
        .take(64)
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    let suffix = if b.len() > 64 { " ..." } else { "" };
    v_flex()
        .gap(px(6.0))
        .child(
            div()
                .text_xs()
                .text_color(muted_fg)
                .child(format!("[{} bytes]", b.len())),
        )
        .child(
            div()
                .w_full()
                .p(px(10.0))
                .border_1()
                .border_color(border)
                .rounded(px(4.0))
                .text_xs()
                .text_color(fg)
                .font_family("monospace")
                .child(format!("{preview}{suffix}")),
        )
}

fn list_block(
    items: &[RedisValue],
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    border: gpui::Hsla,
) -> impl IntoElement {
    let mut rows = v_flex()
        .w_full()
        .gap(px(0.0))
        .border_1()
        .border_color(border)
        .rounded(px(4.0));
    for (i, item) in items.iter().enumerate() {
        rows = rows.child(
            h_flex()
                .w_full()
                .px(px(8.0))
                .py(px(6.0))
                .border_b_1()
                .border_color(border)
                .gap(px(8.0))
                .child(
                    div()
                        .w(px(40.0))
                        .text_xs()
                        .text_color(muted_fg)
                        .flex_none()
                        .child(format!("{i}")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(fg)
                        .font_family("monospace")
                        .child(item.display_preview(256)),
                ),
        );
    }
    rows
}

impl KeyDetailPanel {
    /// 标量值（String / Bytes）渲染：顶部工具条（Gzip 提示 + 4 tab + 编辑按钮）+ 内容区
    #[allow(clippy::too_many_arguments)]
    fn render_scalar_with_tabs(
        &self,
        key: &str,
        v: &RedisValue,
        fg: gpui::Hsla,
        muted_fg: gpui::Hsla,
        accent: gpui::Hsla,
        border: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        // 取原始字节流（用于 Gzip 检测 + 各 tab 渲染）
        let raw_bytes: Vec<u8> = match v {
            RedisValue::Text(s) => s.as_bytes().to_vec(),
            RedisValue::Bytes(b) => b.clone(),
            _ => Vec::new(),
        };

        // 自动 Gzip 解压（成功则用解压结果替代原 bytes 渲染）
        let (display_bytes, gzip_hint) = match value_display::try_decompress_gzip(&raw_bytes) {
            Some(decoded) => {
                let hint = format!(
                    "🗜️ 检测到 Gzip 压缩，已自动解压（原 {} bytes → {} bytes）",
                    raw_bytes.len(),
                    decoded.len()
                );
                (decoded, Some(hint))
            }
            None => (raw_bytes.clone(), None),
        };

        // 按当前 view_mode 渲染主内容文本
        let mode = self.view_mode;
        let content_text = match v {
            RedisValue::Text(_) => {
                // Text 已 utf-8；解压后再判：解压后字节看起来 utf-8 就当 text；否则当 bytes
                match std::str::from_utf8(&display_bytes) {
                    Ok(s) => value_display::render_text(s, mode),
                    Err(_) => value_display::render_bytes(&display_bytes, mode),
                }
            }
            _ => value_display::render_bytes(&display_bytes, mode),
        };

        // 4 tab：当前模式高亮
        let mut accent_tint = accent;
        accent_tint.a = 0.10;
        let tabs_row = {
            let mut row = h_flex().items_center().gap(px(4.0));
            for m in ViewMode::all() {
                let m_val = *m;
                let is_active = mode == m_val;
                let id = SharedString::from(format!("vm-tab-{}", m.label()));
                let mut btn = div()
                    .id(id)
                    .px(px(10.0))
                    .py(px(4.0))
                    .rounded(px(4.0))
                    .text_xs()
                    .cursor_pointer()
                    .child(m.label());
                if is_active {
                    btn = btn.bg(accent_tint).text_color(accent);
                } else {
                    btn = btn
                        .text_color(muted_fg)
                        .hover(|this| this.opacity(0.75))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.set_view_mode(m_val, cx);
                        }));
                }
                row = row.child(btn);
            }
            row
        };

        // 编辑按钮（仅 Text 类型）
        let edit_btn: Option<gpui::AnyElement> = match v {
            RedisValue::Text(s) => {
                let key_for_emit = key.to_string();
                let text_for_emit = s.clone();
                Some(
                    Button::new("redis-string-edit")
                        .outline()
                        .small()
                        .label("编辑值")
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(KeyDetailEvent::RequestEditValue(
                                key_for_emit.clone(),
                                text_for_emit.clone(),
                            ));
                        }))
                        .into_any_element(),
                )
            }
            _ => None,
        };

        v_flex()
            .w_full()
            .gap(px(8.0))
            // 工具条：tabs + 编辑按钮（右对齐）
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .child(tabs_row)
                    .child(div().flex_1())
                    .when_some(edit_btn, |this, b| this.child(b)),
            )
            // Gzip 提示
            .when_some(gzip_hint, |this, hint| {
                this.child(
                    div()
                        .px(px(10.0))
                        .py(px(6.0))
                        .text_xs()
                        .text_color(muted_fg)
                        .border_1()
                        .border_color(border)
                        .rounded(px(4.0))
                        .child(hint),
                )
            })
            // 内容（多行文本，monospace，可滚动靠外层）
            .child(
                div()
                    .w_full()
                    .p(px(10.0))
                    .border_1()
                    .border_color(border)
                    .rounded(px(4.0))
                    .text_sm()
                    .text_color(fg)
                    .font_family("monospace")
                    .child(content_text),
            )
    }
}

/// MEMORY USAGE 显示 chip：未估算时显示 [字节数] 按钮 → 触发 estimate_size；
/// 已估算时显示具体字节数（人类可读单位）
fn render_size_chip(
    bytes: Option<u64>,
    estimating: bool,
    muted_fg: gpui::Hsla,
    accent: gpui::Hsla,
    cx: &mut Context<KeyDetailPanel>,
) -> impl IntoElement + use<> {
    if let Some(n) = bytes {
        let label = format!("{}（{}）", human_readable_bytes(n), n);
        div()
            .id("size-result")
            .text_color(muted_fg)
            .child(format!("📊 {label}"))
            .into_any_element()
    } else if estimating {
        div()
            .id("size-loading")
            .text_color(muted_fg)
            .child("📊 估算中...")
            .into_any_element()
    } else {
        div()
            .id("size-trigger")
            .text_color(accent)
            .cursor_pointer()
            .hover(|this| this.opacity(0.75))
            .child("📊 估算大小")
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.estimate_size(cx)))
            .into_any_element()
    }
}

fn human_readable_bytes(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = n as f64;
    let mut idx = 0;
    while size >= 1024.0 && idx < UNITS.len() - 1 {
        size /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{n} B")
    } else {
        format!("{size:.2} {}", UNITS[idx])
    }
}

/// List 块渲染（带 [🗑] 行内删除按钮）
fn render_list_block(
    panel: &mut Context<KeyDetailPanel>,
    items: &[RedisValue],
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    accent: gpui::Hsla,
    border: gpui::Hsla,
) -> impl IntoElement + use<> {
    let mut rows = v_flex()
        .w_full()
        .gap(px(0.0))
        .border_1()
        .border_color(border)
        .rounded(px(4.0));
    for (i, item) in items.iter().enumerate() {
        let preview = item.display_preview(256);
        let raw_value = match item {
            RedisValue::Text(s) => s.clone(),
            other => other.display_preview(8192),
        };
        let del_id = SharedString::from(format!("list-del-{i}"));
        rows = rows.child(
            h_flex()
                .w_full()
                .px(px(8.0))
                .py(px(6.0))
                .border_b_1()
                .border_color(border)
                .gap(px(8.0))
                .child(
                    div()
                        .w(px(40.0))
                        .text_xs()
                        .text_color(muted_fg)
                        .flex_none()
                        .child(format!("{i}")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(fg)
                        .font_family("monospace")
                        .child(preview),
                )
                .child(
                    div()
                        .id(del_id)
                        .text_xs()
                        .text_color(accent)
                        .cursor_pointer()
                        .hover(|this| this.opacity(0.7))
                        .child("删除")
                        .on_click(panel.listener(move |this, _: &ClickEvent, _, cx| {
                            this.delete_list_element(raw_value.clone(), cx);
                        })),
                ),
        );
    }
    rows
}

/// Set 块渲染（带 [🗑] 行内删除按钮）
fn render_set_block(
    panel: &mut Context<KeyDetailPanel>,
    items: &[RedisValue],
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    accent: gpui::Hsla,
    border: gpui::Hsla,
) -> impl IntoElement + use<> {
    let mut rows = v_flex()
        .w_full()
        .gap(px(0.0))
        .border_1()
        .border_color(border)
        .rounded(px(4.0));
    for (i, item) in items.iter().enumerate() {
        let preview = item.display_preview(256);
        let raw_member = match item {
            RedisValue::Text(s) => s.clone(),
            other => other.display_preview(8192),
        };
        let del_id = SharedString::from(format!("set-del-{i}"));
        rows = rows.child(
            h_flex()
                .w_full()
                .px(px(8.0))
                .py(px(6.0))
                .border_b_1()
                .border_color(border)
                .gap(px(8.0))
                .child(
                    div()
                        .w(px(40.0))
                        .text_xs()
                        .text_color(muted_fg)
                        .flex_none()
                        .child(format!("{i}")),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(fg)
                        .font_family("monospace")
                        .child(preview),
                )
                .child(
                    div()
                        .id(del_id)
                        .text_xs()
                        .text_color(accent)
                        .cursor_pointer()
                        .hover(|this| this.opacity(0.7))
                        .child("删除")
                        .on_click(panel.listener(move |this, _: &ClickEvent, _, cx| {
                            this.delete_set_element(raw_member.clone(), cx);
                        })),
                ),
        );
    }
    rows
}

/// ZSet 块渲染（带 [✎ score][🗑] 行内按钮）
#[allow(clippy::too_many_arguments)]
fn render_zset_block(
    panel: &mut Context<KeyDetailPanel>,
    key: String,
    pairs: &[(RedisValue, f64)],
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    accent: gpui::Hsla,
    border: gpui::Hsla,
) -> impl IntoElement + use<> {
    let mut rows = v_flex()
        .w_full()
        .gap(px(0.0))
        .border_1()
        .border_color(border)
        .rounded(px(4.0));
    for (i, (m, score)) in pairs.iter().enumerate() {
        let preview = m.display_preview(256);
        let raw_member = match m {
            RedisValue::Text(s) => s.clone(),
            other => other.display_preview(8192),
        };
        let score_str = format!("{score:.6}");
        let key_for_edit = key.clone();
        let raw_for_edit = raw_member.clone();
        let raw_for_del = raw_member.clone();
        let edit_id = SharedString::from(format!("zset-edit-{i}"));
        let del_id = SharedString::from(format!("zset-del-{i}"));
        rows = rows.child(
            h_flex()
                .w_full()
                .px(px(8.0))
                .py(px(6.0))
                .border_b_1()
                .border_color(border)
                .gap(px(8.0))
                .child(
                    div()
                        .w(px(80.0))
                        .text_xs()
                        .text_color(muted_fg)
                        .flex_none()
                        .child(score_str.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(fg)
                        .font_family("monospace")
                        .child(preview),
                )
                .child(
                    h_flex()
                        .gap(px(10.0))
                        .flex_none()
                        .child(
                            div()
                                .id(edit_id)
                                .text_xs()
                                .text_color(accent)
                                .cursor_pointer()
                                .hover(|this| this.opacity(0.7))
                                .child("改 score")
                                .on_click(panel.listener(move |_, _: &ClickEvent, _, cx| {
                                    cx.emit(KeyDetailEvent::RequestEditZSetScore(
                                        key_for_edit.clone(),
                                        raw_for_edit.clone(),
                                        score_str.clone(),
                                    ));
                                })),
                        )
                        .child(
                            div()
                                .id(del_id)
                                .text_xs()
                                .text_color(accent)
                                .cursor_pointer()
                                .hover(|this| this.opacity(0.7))
                                .child("删除")
                                .on_click(panel.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.delete_zset_member(raw_for_del.clone(), cx);
                                })),
                        ),
                ),
        );
    }
    rows
}

/// Hash 块渲染（KeyDetailPanel 方法版）：每行带 [✎][🗑] 操作按钮
///
/// `+ use<>`：与 key_tree::render_node_row 同款，避免与 cx.listener 借用冲突
fn render_hash_block(
    panel: &mut Context<KeyDetailPanel>,
    key: String,
    pairs: &[(String, RedisValue)],
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    accent: gpui::Hsla,
    border: gpui::Hsla,
) -> impl IntoElement + use<> {
    let mut rows = v_flex()
        .w_full()
        .gap(px(0.0))
        .border_1()
        .border_color(border)
        .rounded(px(4.0));
    for (idx, (f, v)) in pairs.iter().enumerate() {
        let field_name = f.clone();
        let value_preview = v.display_preview(256);
        // 编辑用的"原始文本"取最完整可读形态；二进制 Bytes 走 hex 预览
        let value_for_edit = match v {
            RedisValue::Text(s) => s.clone(),
            other => other.display_preview(8192),
        };

        let key_for_edit = key.clone();
        let field_for_edit = field_name.clone();
        let value_for_edit_clone = value_for_edit.clone();
        let key_for_del = key.clone();
        let field_for_del = field_name.clone();

        let edit_id = SharedString::from(format!("hash-edit-{idx}"));
        let del_id = SharedString::from(format!("hash-del-{idx}"));

        rows = rows.child(
            h_flex()
                .w_full()
                .px(px(8.0))
                .py(px(6.0))
                .border_b_1()
                .border_color(border)
                .gap(px(8.0))
                .child(
                    div()
                        .w(px(160.0))
                        .text_xs()
                        .text_color(muted_fg)
                        .flex_none()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(field_name),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(fg)
                        .font_family("monospace")
                        .child(value_preview),
                )
                // 行尾按钮：accent 文字按钮（与连接列表 UX 一致）
                .child(
                    h_flex()
                        .gap(px(10.0))
                        .flex_none()
                        .child(
                            div()
                                .id(edit_id)
                                .text_xs()
                                .text_color(accent)
                                .cursor_pointer()
                                .hover(|this| this.opacity(0.7))
                                .child("编辑")
                                .on_click(panel.listener(move |_, _: &ClickEvent, _, cx| {
                                    cx.emit(KeyDetailEvent::RequestEditHashField(
                                        key_for_edit.clone(),
                                        field_for_edit.clone(),
                                        value_for_edit_clone.clone(),
                                    ));
                                })),
                        )
                        .child(
                            div()
                                .id(del_id)
                                .text_xs()
                                .text_color(accent)
                                .cursor_pointer()
                                .hover(|this| this.opacity(0.7))
                                .child("删除")
                                .on_click(panel.listener(move |this, _: &ClickEvent, _, cx| {
                                    let _ = key_for_del.clone();
                                    this.delete_hash_field(field_for_del.clone(), cx);
                                })),
                        ),
                ),
        );
    }
    rows
}

/// Stream 块渲染（KeyDetailPanel 方法版）：每条 entry 显示 ID + 字段对 + [删除]
fn render_stream_block(
    panel: &mut Context<KeyDetailPanel>,
    entries: &[StreamEntry],
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    accent: gpui::Hsla,
    border: gpui::Hsla,
) -> impl IntoElement + use<> {
    let mut blocks = v_flex().w_full().gap(px(8.0));
    for (idx, e) in entries.iter().enumerate() {
        let mut fields = v_flex().w_full().gap(px(2.0)).pl(px(12.0));
        for (k, v) in &e.fields {
            fields = fields.child(
                h_flex()
                    .w_full()
                    .gap(px(8.0))
                    .child(
                        div()
                            .w(px(140.0))
                            .text_xs()
                            .text_color(muted_fg)
                            .flex_none()
                            .child(k.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(fg)
                            .font_family("monospace")
                            .child(v.clone()),
                    ),
            );
        }
        let entry_id = e.id.clone();
        let id_for_del = entry_id.clone();
        let del_btn_id = SharedString::from(format!("stream-del-{idx}"));
        blocks = blocks.child(
            v_flex()
                .w_full()
                .p(px(8.0))
                .border_1()
                .border_color(border)
                .rounded(px(4.0))
                .gap(px(4.0))
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(fg)
                                .child(entry_id),
                        )
                        .child(
                            div()
                                .id(del_btn_id)
                                .text_xs()
                                .text_color(accent)
                                .cursor_pointer()
                                .hover(|this| this.opacity(0.7))
                                .child("删除")
                                .on_click(panel.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.delete_stream_entry(id_for_del.clone(), cx);
                                })),
                        ),
                )
                .child(fields),
        );
    }
    blocks
}

/// 把毫秒数格式化为人类可读
fn format_ttl_ms(ms: i64) -> String {
    let secs = ms / 1000;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else if secs < 86_400 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}d {}h", secs / 86_400, (secs % 86_400) / 3600)
    }
}

/// 简单的并发 await 两个 future（不引入额外依赖）
/// 借 GPUI 已有的 futures crate（workspace 默认包含）
async fn futures_join<A, B, RA, RB>(a: A, b: B) -> (RA, RB)
where
    A: std::future::Future<Output = RA>,
    B: std::future::Future<Output = RB>,
{
    use futures::future::join;
    join(a, b).await
}
