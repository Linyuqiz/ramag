//! 新建 Key 弹窗：选类型 → 填 key/value/TTL → 提交后调 Redis 命令写入
//!
//! 支持类型与命令映射：
//! - String → `SET key value [EX ttl]`
//! - List   → `RPUSH key v1 v2 ...`（每行一个元素）
//! - Set    → `SADD key m1 m2 ...`（每行一个元素）
//! - Hash   → `HSET key f1 v1 f2 v2 ...`（每行 `field value`，空格分隔）
//! - ZSet   → `ZADD key s1 m1 s2 m2 ...`（每行 `score member`，空格分隔）
//!
//! Stream 留到 Stage 17（XADD 字段语义稍复杂）

use std::sync::Arc;

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, div, prelude::*, px,
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

#[derive(Debug, Clone)]
pub enum KeyCreateEvent {
    /// 创建成功，返回 key 名（让上层刷新树并选中新 key）
    Created(String),
    /// 用户取消
    Cancelled,
}

#[derive(Debug, Clone)]
enum SubmitState {
    Idle,
    Submitting,
    Failed(String),
}

/// 可创建的类型（前 5 个基础类型；Stream 留待后续）
const CREATE_TYPES: &[RedisType] = &[
    RedisType::String,
    RedisType::List,
    RedisType::Hash,
    RedisType::Set,
    RedisType::ZSet,
];

pub struct KeyCreateForm {
    service: Arc<RedisService>,
    config: ConnectionConfig,
    db: u8,
    selected_type: RedisType,
    key_name: Entity<InputState>,
    /// 多行输入（按 type 显示不同 placeholder）
    value: Entity<InputState>,
    ttl_secs: Entity<InputState>,
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
        let value = cx.new(|cx| {
            // multi_line 让输入框支持多行（List/Hash/ZSet 每行一项）
            InputState::new(window, cx)
                .multi_line(true)
                .placeholder("（按类型填写：详见下方提示）")
        });
        let ttl_secs = cx.new(|cx| InputState::new(window, cx).placeholder("（可选，秒）"));

        Self {
            service,
            config,
            db,
            selected_type: RedisType::String,
            key_name,
            value,
            ttl_secs,
            state: SubmitState::Idle,
        }
    }

    fn select_type(&mut self, t: RedisType, cx: &mut Context<Self>) {
        if self.selected_type == t {
            return;
        }
        self.selected_type = t;
        cx.notify();
    }

    fn validate_and_build_argv(&self, cx: &gpui::App) -> Result<Vec<String>, String> {
        let key = self.key_name.read(cx).value().trim().to_string();
        if key.is_empty() {
            return Err("请填写 Key 名".into());
        }
        let raw_value = self.value.read(cx).value().to_string();

        match self.selected_type {
            RedisType::String => {
                // 允许空值（SET key ""）
                Ok(vec!["SET".into(), key, raw_value])
            }
            RedisType::List => {
                let elems = parse_lines(&raw_value);
                if elems.is_empty() {
                    return Err("List 至少需要 1 个元素（每行一个）".into());
                }
                let mut argv = vec!["RPUSH".into(), key];
                argv.extend(elems);
                Ok(argv)
            }
            RedisType::Set => {
                let elems = parse_lines(&raw_value);
                if elems.is_empty() {
                    return Err("Set 至少需要 1 个元素（每行一个）".into());
                }
                let mut argv = vec!["SADD".into(), key];
                argv.extend(elems);
                Ok(argv)
            }
            RedisType::Hash => {
                let pairs = parse_kv_pairs(&raw_value)?;
                if pairs.is_empty() {
                    return Err("Hash 至少需要 1 个字段（每行 `field value`）".into());
                }
                let mut argv = vec!["HSET".into(), key];
                for (f, v) in pairs {
                    argv.push(f);
                    argv.push(v);
                }
                Ok(argv)
            }
            RedisType::ZSet => {
                let pairs = parse_score_member(&raw_value)?;
                if pairs.is_empty() {
                    return Err("ZSet 至少需要 1 个成员（每行 `score member`）".into());
                }
                let mut argv = vec!["ZADD".into(), key];
                for (s, m) in pairs {
                    argv.push(s);
                    argv.push(m);
                }
                Ok(argv)
            }
            RedisType::Stream | RedisType::None => {
                Err("Stream 类型暂未在 Stage 16 支持，请用命令面板执行 XADD".into())
            }
        }
    }

    fn handle_create(&mut self, cx: &mut Context<Self>) {
        let argv = match self.validate_and_build_argv(cx) {
            Ok(v) => v,
            Err(e) => {
                self.state = SubmitState::Failed(e);
                cx.notify();
                return;
            }
        };
        let key = self.key_name.read(cx).value().trim().to_string();
        // TTL：先解析，再单独发 EXPIRE（避免 SET 加 EX 与其他命令路径不一致）
        let ttl_input = self.ttl_secs.read(cx).value().trim().to_string();
        let ttl: Option<i64> = if ttl_input.is_empty() {
            None
        } else {
            match ttl_input.parse() {
                Ok(n) if n > 0 => Some(n),
                _ => {
                    self.state = SubmitState::Failed("TTL 必须是正整数（秒）".into());
                    cx.notify();
                    return;
                }
            }
        };

        self.state = SubmitState::Submitting;
        cx.notify();

        let svc = self.service.clone();
        let config = self.config.clone();
        let db = self.db;
        cx.spawn(async move |this, cx| {
            let write_result = svc.execute_command(&config, db, argv).await;
            // 如成功且需要 TTL，再发 EXPIRE
            let final_result = match write_result {
                Ok(_) => {
                    if let Some(ts) = ttl {
                        match svc.set_ttl(&config, db, &key, Some(ts)).await {
                            Ok(_) => Ok(()),
                            Err(e) => Err(format!("写入成功但 TTL 设置失败：{e}")),
                        }
                    } else {
                        Ok(())
                    }
                }
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
}

impl Render for KeyCreateForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let accent = theme.accent;
        let border = theme.border;
        let secondary_bg = theme.secondary;

        let selected_type = self.selected_type;
        let mut accent_tint = accent;
        accent_tint.a = 0.10;
        let mut accent_border = accent;
        accent_border.a = 0.55;

        // 类型选择器：5 个按钮等分
        let mut type_row = h_flex().w_full().items_center().gap(px(8.0));
        for t in CREATE_TYPES {
            let is_selected = selected_type == *t;
            let label = t.label();
            let btn_id = SharedString::from(format!("ktype-{}", t.as_scan_arg()));

            let mut btn = h_flex()
                .id(btn_id)
                .flex_1()
                .min_w_0()
                .items_center()
                .justify_center()
                .px(px(8.0))
                .py(px(7.0))
                .rounded_md()
                .border_1()
                .text_sm()
                .child(label);

            if is_selected {
                btn = btn
                    .bg(accent_tint)
                    .border_color(accent_border)
                    .text_color(accent);
            } else {
                let kind = *t;
                btn = btn
                    .bg(secondary_bg)
                    .border_color(border)
                    .text_color(fg)
                    .cursor_pointer()
                    .hover(move |this| this.border_color(accent_border))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.select_type(kind, cx);
                    }));
            }
            type_row = type_row.child(btn);
        }

        // 值字段提示文案
        let value_hint = match selected_type {
            RedisType::String => "字符串值（任意文本）",
            RedisType::List => "每行一个元素",
            RedisType::Set => "每行一个元素（自动去重）",
            RedisType::Hash => "每行 `field value`，空格分隔",
            RedisType::ZSet => "每行 `score member`，空格分隔",
            _ => "",
        };

        let err_msg = match &self.state {
            SubmitState::Idle | SubmitState::Submitting => None,
            SubmitState::Failed(s) => Some(s.clone()),
        };

        v_flex()
            .w_full()
            .gap(px(18.0))
            .pt(px(4.0))
            .pb(px(4.0))
            .child(
                v_flex()
                    .gap(px(8.0))
                    .child(section_title("类型", muted_fg))
                    .child(type_row),
            )
            .child(
                v_flex()
                    .gap(px(12.0))
                    .child(section_title("Key 名", muted_fg))
                    .child(div().w_full().child(Input::new(&self.key_name))),
            )
            .child(
                v_flex()
                    .gap(px(8.0))
                    .child(section_title("值", muted_fg))
                    .child(div().text_xs().text_color(muted_fg).child(value_hint))
                    .child(div().w_full().h(px(160.0)).child(Input::new(&self.value))),
            )
            .child(
                v_flex()
                    .gap(px(8.0))
                    .child(section_title("TTL（秒，可选）", muted_fg))
                    .child(div().w(px(180.0)).child(Input::new(&self.ttl_secs))),
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
                                    .label(if matches!(self.state, SubmitState::Submitting) {
                                        "创建中..."
                                    } else {
                                        "创建"
                                    })
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

// ===== 解析辅助 =====

fn parse_lines(raw: &str) -> Vec<String> {
    raw.lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// "field value" 每行 → (field, value) 列表；空行跳过
fn parse_kv_pairs(raw: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        match line.split_once(' ') {
            Some((f, v)) => {
                let f = f.trim();
                if f.is_empty() {
                    return Err(format!("第 {} 行：field 为空", idx + 1));
                }
                out.push((f.to_string(), v.trim_start().to_string()));
            }
            None => {
                return Err(format!(
                    "第 {} 行格式错误：需要 `field value`（空格分隔）",
                    idx + 1
                ));
            }
        }
    }
    Ok(out)
}

/// "score member" 每行 → (score, member) 列表
fn parse_score_member(raw: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let (score_str, member) = match line.split_once(' ') {
            Some(p) => p,
            None => {
                return Err(format!(
                    "第 {} 行格式错误：需要 `score member`（空格分隔）",
                    idx + 1
                ));
            }
        };
        score_str
            .trim()
            .parse::<f64>()
            .map_err(|_| format!("第 {} 行：score 必须是数字，实得 `{score_str}`", idx + 1))?;
        let member = member.trim_start().to_string();
        if member.is_empty() {
            return Err(format!("第 {} 行：member 为空", idx + 1));
        }
        out.push((score_str.trim().to_string(), member));
    }
    Ok(out)
}

fn section_title(text: &str, muted_fg: gpui::Hsla) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap(px(8.0))
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(muted_fg)
                .child(text.to_string()),
        )
        .child(div().flex_1().h(px(1.0)).bg(muted_fg).opacity(0.12))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lines_skips_empty() {
        // 含 CRLF（Windows 风格）+ 空行 + 普通行
        let out = parse_lines("a\r\n\r\nb\nc\r\n");
        assert_eq!(out, vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_kv_basic() {
        let out = parse_kv_pairs("name alice\nage 30").unwrap();
        assert_eq!(
            out,
            vec![("name".into(), "alice".into()), ("age".into(), "30".into())]
        );
    }

    #[test]
    fn parse_kv_value_with_space_keeps_remainder() {
        let out = parse_kv_pairs("desc hello world").unwrap();
        assert_eq!(out, vec![("desc".into(), "hello world".into())]);
    }

    #[test]
    fn parse_kv_missing_value_errors() {
        assert!(parse_kv_pairs("only_field").is_err());
    }

    #[test]
    fn parse_score_member_works() {
        let out = parse_score_member("1.5 alice\n2 bob").unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, "1.5");
        assert_eq!(out[0].1, "alice");
    }

    #[test]
    fn parse_score_invalid() {
        assert!(parse_score_member("not_a_number alice").is_err());
    }
}
