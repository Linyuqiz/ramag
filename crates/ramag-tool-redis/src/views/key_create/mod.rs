//! 新建 Key 对话框
//!
//! 对外保持与旧版 `key_create.rs` 同名公共 API（`KeyCreateForm::new` + `KeyCreateEvent`），
//! 内部按类型拆为 5 个结构化子编辑器，避免文本格式记忆负担。
//!
//! # 类型与命令
//!
//! - String → `SET key value`
//! - List   → `RPUSH/LPUSH key v1 v2 ...`（默认 RPUSH，可在编辑器内切换）
//! - Set    → `SADD key m1 m2 ...`（提交时客户端去重，保留首次出现顺序）
//! - Hash   → `HSET key f1 v1 f2 v2 ...`
//! - ZSet   → `ZADD key s1 m1 s2 m2 ...`
//! - Stream → `XADD key * f1 v1 ...`（ID 由服务端生成）
//!
//! # TTL
//!
//! 写入命令成功后单独 EXPIRE，避免不同命令对 EX/EXAT 选项支持不一致

use std::collections::HashSet;
use std::sync::Arc;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, Hsla, IntoElement, ParentElement,
    Render, SharedString, Styled, Window, div, hsla, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    v_flex,
};
use ramag_app::RedisService;
use ramag_domain::entities::{ConnectionConfig, RedisType};
use tracing::{error, info};

use crate::views::lines_editor::{LinesEditor, LinesKind, PushDir};
use crate::views::pairs_editor::{PairsEditor, PairsKind};
use crate::views::ttl_picker::TtlPicker;

#[derive(Debug, Clone)]
pub enum KeyCreateEvent {
    /// 创建成功，返回 key 名（让上层刷新树并选中新 key）
    Created(String),
    Cancelled,
}

#[derive(Debug, Clone)]
enum SubmitState {
    Idle,
    Submitting,
    Failed(String),
}

/// 类型选择按钮的展示顺序
const CREATE_TYPES: &[RedisType] = &[
    RedisType::String,
    RedisType::List,
    RedisType::Hash,
    RedisType::Set,
    RedisType::ZSet,
    RedisType::Stream,
];

pub struct KeyCreateForm {
    service: Arc<RedisService>,
    config: ConnectionConfig,
    db: u8,
    selected_type: RedisType,
    key_name: Entity<InputState>,
    string_input: Entity<InputState>,
    list_editor: Entity<LinesEditor>,
    set_editor: Entity<LinesEditor>,
    hash_editor: Entity<PairsEditor>,
    zset_editor: Entity<PairsEditor>,
    stream_editor: Entity<PairsEditor>,
    ttl_picker: Entity<TtlPicker>,
    state: SubmitState,
}

impl EventEmitter<KeyCreateEvent> for KeyCreateForm {}

impl KeyCreateForm {
    pub fn new(
        service: Arc<RedisService>,
        config: ConnectionConfig,
        db: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let key_name = cx.new(|cx| InputState::new(window, cx).placeholder("如 user:1001:cache"));
        // String multi_line input 的高度必须通过 Input view 的 .h() 设置，
        // 不能靠外层 div h() ——后者在 multi_line 渲染路径中被忽略
        let string_input = cx.new(|cx| {
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("字符串值（任意文本，可多行）")
        });
        let list_editor = cx.new(|cx| LinesEditor::new(LinesKind::List, window, cx));
        let set_editor = cx.new(|cx| LinesEditor::new(LinesKind::Set, window, cx));
        let hash_editor = cx.new(|cx| PairsEditor::new(PairsKind::Hash, window, cx));
        let zset_editor = cx.new(|cx| PairsEditor::new(PairsKind::ZSet, window, cx));
        let stream_editor = cx.new(|cx| PairsEditor::new(PairsKind::Stream, window, cx));
        let ttl_picker = cx.new(|cx| TtlPicker::new(window, cx));

        Self {
            service,
            config,
            db,
            selected_type: RedisType::String,
            key_name,
            string_input,
            list_editor,
            set_editor,
            hash_editor,
            zset_editor,
            stream_editor,
            ttl_picker,
            state: SubmitState::Idle,
        }
    }

    fn select_type(&mut self, t: RedisType, cx: &mut Context<Self>) {
        if self.selected_type != t {
            self.selected_type = t;
            // 切换类型时清掉旧错误，避免误导
            if let SubmitState::Failed(_) = self.state {
                self.state = SubmitState::Idle;
            }
            cx.notify();
        }
    }

    /// 校验 + 拼 argv + 拼 TTL
    fn build_argv_and_ttl(&self, cx: &gpui::App) -> Result<(Vec<String>, Option<i64>), String> {
        let key = self.key_name.read(cx).value().trim().to_string();
        if key.is_empty() {
            return Err("请填写 Key 名".into());
        }

        let argv: Vec<String> = match self.selected_type {
            RedisType::String => {
                let v = self.string_input.read(cx).value().to_string();
                vec!["SET".into(), key.clone(), v]
            }
            RedisType::List => {
                let editor = self.list_editor.read(cx);
                let elems = editor.collect(cx);
                if elems.is_empty() {
                    return Err("List 至少需要 1 个元素".into());
                }
                let cmd = match editor.push_dir() {
                    PushDir::Tail => "RPUSH",
                    PushDir::Head => "LPUSH",
                };
                let mut argv = vec![cmd.into(), key.clone()];
                argv.extend(elems);
                argv
            }
            RedisType::Set => {
                let elems = self.set_editor.read(cx).collect(cx);
                if elems.is_empty() {
                    return Err("Set 至少需要 1 个成员".into());
                }
                let mut seen: HashSet<String> = HashSet::new();
                let dedup: Vec<String> = elems
                    .into_iter()
                    .filter(|s| seen.insert(s.clone()))
                    .collect();
                let mut argv = vec!["SADD".into(), key.clone()];
                argv.extend(dedup);
                argv
            }
            RedisType::Hash => {
                let pairs = self.hash_editor.read(cx).collect(cx)?;
                if pairs.is_empty() {
                    return Err("Hash 至少需要 1 个字段".into());
                }
                let mut argv = vec!["HSET".into(), key.clone()];
                for (f, v) in pairs {
                    argv.push(f);
                    argv.push(v);
                }
                argv
            }
            RedisType::ZSet => {
                let pairs = self.zset_editor.read(cx).collect(cx)?;
                if pairs.is_empty() {
                    return Err("ZSet 至少需要 1 个成员".into());
                }
                let mut argv = vec!["ZADD".into(), key.clone()];
                for (s, m) in pairs {
                    argv.push(s);
                    argv.push(m);
                }
                argv
            }
            RedisType::Stream => {
                let pairs = self.stream_editor.read(cx).collect(cx)?;
                if pairs.is_empty() {
                    return Err("Stream 至少需要 1 个字段".into());
                }
                let mut argv = vec!["XADD".into(), key.clone(), "*".into()];
                for (f, v) in pairs {
                    argv.push(f);
                    argv.push(v);
                }
                argv
            }
            RedisType::None => return Err("未知类型".into()),
        };

        let ttl = self.ttl_picker.read(cx).collect(cx)?;
        Ok((argv, ttl))
    }

    fn handle_create(&mut self, cx: &mut Context<Self>) {
        let (argv, ttl) = match self.build_argv_and_ttl(cx) {
            Ok(t) => t,
            Err(e) => {
                self.state = SubmitState::Failed(e);
                cx.notify();
                return;
            }
        };
        let key = self.key_name.read(cx).value().trim().to_string();
        let intended_type = self.selected_type;

        self.state = SubmitState::Submitting;
        cx.notify();

        let svc = self.service.clone();
        let config = self.config.clone();
        let db = self.db;
        cx.spawn(async move |this, cx| {
            // 第 1 步：预检 key 类型，拒绝跨类型覆盖
            // - None    → 不存在，安全继续
            // - 同类型  → 允许（Redis 行为：String SET 覆盖；List/Hash/Set/ZSet 合并；Stream XADD 追加）
            // - 不同类型 → 拒绝，避免 WRONGTYPE 错误前已知拦下
            let precheck = svc.key_type(&config, db, &key).await;
            if let Ok(existing) = precheck
                && existing != RedisType::None
                && existing != intended_type
            {
                let msg = format!(
                    "已存在「{}」类型的 key「{key}」，不能用「{}」类型覆盖。请先删除原 key 或换名。",
                    existing.label(),
                    intended_type.label(),
                );
                let _ = this.update(cx, |this, cx| {
                    error!(error = %msg, "create key precheck failed: type conflict");
                    this.state = SubmitState::Failed(msg);
                    cx.notify();
                });
                return;
            }

            // 第 2 步：写入命令
            let write_result = svc.execute_command(&config, db, argv).await;
            let final_result = match write_result {
                Ok(_) => match ttl {
                    Some(ts) => match svc.set_ttl(&config, db, &key, Some(ts)).await {
                        Ok(_) => Ok(()),
                        Err(e) => Err(format!("写入成功但 TTL 设置失败：{e}")),
                    },
                    None => Ok(()),
                },
                Err(e) => Err(format!("{e}")),
            };
            let _ = this.update(cx, |this, cx| match final_result {
                Ok(_) => {
                    info!(?key, ?ttl, "redis key created");
                    cx.emit(KeyCreateEvent::Created(key.clone()));
                }
                Err(msg) => {
                    error!(error = %msg, "create key failed");
                    this.state = SubmitState::Failed(msg);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn handle_cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(KeyCreateEvent::Cancelled);
    }

    /// 当前类型对应的 editor 元素
    fn render_editor(&self) -> AnyElement {
        match self.selected_type {
            // multi_line 高度走 Input 自己的 .h()（外层 div h() 在 multi_line 渲染中被忽略）
            // 220px 与编辑弹窗 value_edit.rs 同款，避免新建 / 修改两边视觉跳变
            RedisType::String => Input::new(&self.string_input)
                .h(px(220.0))
                .into_any_element(),
            RedisType::List => self.list_editor.clone().into_any_element(),
            RedisType::Set => self.set_editor.clone().into_any_element(),
            RedisType::Hash => self.hash_editor.clone().into_any_element(),
            RedisType::ZSet => self.zset_editor.clone().into_any_element(),
            RedisType::Stream => self.stream_editor.clone().into_any_element(),
            RedisType::None => div().into_any_element(),
        }
    }
}

impl Render for KeyCreateForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let border = theme.border;
        let secondary_bg = theme.secondary;

        let current_color = redis_type_color(self.selected_type);
        let mut card_bg = secondary_bg;
        card_bg.a = 0.45;

        // === 类型 chip 行（6 个等分，前缀类型色 dot） ===
        let mut type_row = h_flex().w_full().items_center().gap(px(6.0));
        for t in CREATE_TYPES {
            let is_selected = self.selected_type == *t;
            let kind = *t;
            let label = t.label();
            let color = redis_type_color(kind);
            let mut tint = color;
            tint.a = 0.12;
            let mut soft_border = color;
            soft_border.a = 0.55;

            let dot = div()
                .w(px(7.0))
                .h(px(7.0))
                .rounded_full()
                .bg(color)
                .flex_none();

            let btn_id = SharedString::from(format!("ktype-{}", t.as_scan_arg()));
            let mut btn = h_flex()
                .id(btn_id)
                .flex_1()
                .min_w_0()
                .items_center()
                .justify_center()
                .gap(px(6.0))
                .px(px(10.0))
                .py(px(8.0))
                .rounded_md()
                .border_1()
                .text_sm()
                .child(dot)
                .child(label);
            if is_selected {
                btn = btn
                    .bg(tint)
                    .border_color(soft_border)
                    .text_color(color)
                    .font_weight(gpui::FontWeight::SEMIBOLD);
            } else {
                btn = btn
                    .bg(secondary_bg)
                    .border_color(border)
                    .text_color(fg)
                    .cursor_pointer()
                    .hover(move |this| this.border_color(soft_border))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.select_type(kind, cx);
                    }));
            }
            type_row = type_row.child(btn);
        }

        let err_msg = match &self.state {
            SubmitState::Idle | SubmitState::Submitting => None,
            SubmitState::Failed(s) => Some(s.clone()),
        };
        let submitting = matches!(self.state, SubmitState::Submitting);
        let value_section_title = format!("{} 值", self.selected_type.label());

        v_flex()
            .w_full()
            .gap(px(18.0))
            .pt(px(2.0))
            .pb(px(2.0))
            .child(
                v_flex()
                    .gap(px(8.0))
                    .child(section_title("Key 名", muted_fg, None))
                    .child(div().w_full().child(Input::new(&self.key_name))),
            )
            .child(
                v_flex()
                    .gap(px(8.0))
                    .child(section_title("类型", muted_fg, None))
                    .child(type_row),
            )
            .child(
                v_flex()
                    .gap(px(10.0))
                    .child(section_title(
                        &value_section_title,
                        muted_fg,
                        Some(current_color),
                    ))
                    .child(
                        div()
                            .w_full()
                            .p(px(14.0))
                            .rounded_md()
                            .border_1()
                            .border_color(border)
                            .bg(card_bg)
                            .child(self.render_editor()),
                    ),
            )
            .child(
                v_flex()
                    .gap(px(8.0))
                    .child(section_title("TTL", muted_fg, None))
                    .child(self.ttl_picker.clone()),
            )
            .child(div().h(px(1.0)).bg(border).my(px(2.0)))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(gpui::red())
                            .child(err_msg.unwrap_or_default()),
                    )
                    .child(
                        h_flex()
                            .gap(px(8.0))
                            .flex_none()
                            .child(
                                Button::new("kc-cancel")
                                    .ghost()
                                    .small()
                                    .label("取消")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.handle_cancel(cx);
                                    })),
                            )
                            .child(
                                Button::new("kc-create")
                                    .primary()
                                    .small()
                                    .label(if submitting { "创建中..." } else { "创建" })
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        if !matches!(this.state, SubmitState::Submitting) {
                                            this.handle_create(cx);
                                        }
                                    })),
                            ),
                    ),
            )
    }
}

/// 通用 section 标题：可选前缀类型色 dot + 标签 + 右侧极淡分隔线
fn section_title(text: &str, muted_fg: Hsla, dot_color: Option<Hsla>) -> impl IntoElement {
    let mut row = h_flex().items_center().gap(px(8.0));
    if let Some(c) = dot_color {
        row = row.child(div().w(px(8.0)).h(px(8.0)).rounded_full().bg(c).flex_none());
    }
    row.child(
        div()
            .text_xs()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(muted_fg)
            .child(text.to_string()),
    )
    .child(div().flex_1().h(px(1.0)).bg(muted_fg).opacity(0.12))
}

/// Redis 类型标志色（与 `key_tree::type_color_solid` 同款，刻意不跨模块复用以避免破坏分层）
fn redis_type_color(t: RedisType) -> Hsla {
    match t {
        RedisType::String => hsla(210.0 / 360.0, 0.6, 0.55, 1.0),
        RedisType::List => hsla(140.0 / 360.0, 0.5, 0.5, 1.0),
        RedisType::Hash => hsla(280.0 / 360.0, 0.55, 0.6, 1.0),
        RedisType::Set => hsla(40.0 / 360.0, 0.85, 0.55, 1.0),
        RedisType::ZSet => hsla(20.0 / 360.0, 0.7, 0.55, 1.0),
        RedisType::Stream => hsla(330.0 / 360.0, 0.55, 0.55, 1.0),
        RedisType::None => hsla(0.0, 0.0, 0.5, 1.0),
    }
}
