//! CLI 命令面板：自由输入 Redis 命令 + 滚动应答历史
//!
//! 仿 redis-cli 的体验：
//! - 底部输入框 + 上方滚动的"命令 + 应答"块流
//! - Enter 提交；上箭头回溯历史命令
//! - 危险命令（FLUSHDB/FLUSHALL/CONFIG SET/DEBUG/SHUTDOWN/KEYS）前端拦截，需二次确认
//! - 应答按 RESP 类型分色显示：成功 fg / 错误 red / 耗时 muted

use std::sync::Arc;

use gpui::{
    ClickEvent, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, SharedString,
    Styled, Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    v_flex,
};
use ramag_app::RedisService;
use ramag_domain::entities::{ConnectionConfig, RedisValue};
use tracing::{error, info};

/// 危险命令前缀集合（不区分大小写）
/// 触发时会弹确认而不是直接执行
const DANGEROUS_PREFIXES: &[&str] = &[
    "FLUSHDB",
    "FLUSHALL",
    "CONFIG SET",
    "DEBUG",
    "SHUTDOWN",
    "KEYS",
    "CLIENT KILL",
    "SCRIPT KILL",
];

/// 单条命令历史条目
#[derive(Debug, Clone)]
struct HistoryEntry {
    command: String,
    response: ResponseKind,
    elapsed_ms: u128,
}

#[derive(Debug, Clone)]
enum ResponseKind {
    /// 应答文本（RedisValue.display_preview 展开）
    Ok(String),
    /// 错误消息
    Err(String),
    /// 等待中（命令已派发，应答未到）
    Pending,
}

#[derive(Debug, Clone)]
pub enum CliEvent {
    /// 触发危险命令拦截（让 Session 弹确认对话框）
    /// (raw_command, argv)
    RequestDangerConfirm(String, Vec<String>),
}

pub struct CliPanel {
    service: Arc<RedisService>,
    config: ConnectionConfig,
    db: u8,
    history: Vec<HistoryEntry>,
    input: Entity<InputState>,
    /// 命令历史（去重；按时间倒序索引）
    cmd_history: Vec<String>,
    /// 当前回溯索引（None = 没在浏览历史）
    cmd_history_idx: Option<usize>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl EventEmitter<CliEvent> for CliPanel {}

impl CliPanel {
    pub fn new(
        service: Arc<RedisService>,
        config: ConnectionConfig,
        db: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("输入 Redis 命令，Enter 执行（如 GET foo）")
        });

        let subs = vec![cx.subscribe_in(
            &input,
            window,
            |this: &mut Self, _, e: &InputEvent, window, cx| {
                // PressEnter 提交（gpui-component InputEvent::PressEnter）
                if matches!(e, InputEvent::PressEnter { .. }) {
                    this.handle_submit(window, cx);
                }
            },
        )];

        Self {
            service,
            config,
            db,
            history: Vec::new(),
            input,
            cmd_history: Vec::new(),
            cmd_history_idx: None,
            _subscriptions: subs,
        }
    }

    /// 切换 DB（外部 Session 调用）
    pub fn set_db(&mut self, db: u8, cx: &mut Context<Self>) {
        self.db = db;
        cx.notify();
    }

    fn handle_submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.input.read(cx).value().trim().to_string();
        if raw.is_empty() {
            return;
        }
        // 解析 argv（朴素：按空白分隔，不处理引号；后续可加 shell-like 解析）
        let argv: Vec<String> = raw.split_whitespace().map(String::from).collect();
        if argv.is_empty() {
            return;
        }

        // 危险命令检查
        if is_dangerous(&raw) {
            cx.emit(CliEvent::RequestDangerConfirm(raw, argv));
            return;
        }

        self.execute(raw, argv, window, cx);
    }

    /// 由 Session 在确认后调用（绕过危险检查）
    pub fn execute_confirmed(
        &mut self,
        raw: String,
        argv: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.execute(raw, argv, window, cx);
    }

    fn execute(
        &mut self,
        raw: String,
        argv: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 加入命令历史（去掉重复的最近一条）
        if self.cmd_history.last().map(String::as_str) != Some(raw.as_str()) {
            self.cmd_history.push(raw.clone());
        }
        self.cmd_history_idx = None;

        // 入栈一个 Pending 行（后续异步替换）
        let entry_idx = self.history.len();
        self.history.push(HistoryEntry {
            command: raw.clone(),
            response: ResponseKind::Pending,
            elapsed_ms: 0,
        });

        // 清空输入框（同步，需 window）
        self.input
            .update(cx, |state, cx| state.set_value("", window, cx));
        cx.notify();

        let svc = self.service.clone();
        let config = self.config.clone();
        let db = self.db;
        let start = std::time::Instant::now();
        cx.spawn(async move |this, cx| {
            let result = svc.execute_command(&config, db, argv).await;
            let elapsed = start.elapsed().as_millis();
            let _ = this.update(cx, |this, cx| {
                if let Some(entry) = this.history.get_mut(entry_idx) {
                    entry.elapsed_ms = elapsed;
                    entry.response = match result {
                        Ok(v) => {
                            info!(elapsed_ms = elapsed, "cli command ok");
                            ResponseKind::Ok(format_response(&v))
                        }
                        Err(e) => {
                            error!(error = %e, "cli command failed");
                            ResponseKind::Err(format!("{e}"))
                        }
                    };
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn clear_history(&mut self, cx: &mut Context<Self>) {
        self.history.clear();
        cx.notify();
    }
}

/// 格式化 RedisValue 为多行可读文本（仿 redis-cli 输出）
fn format_response(v: &RedisValue) -> String {
    match v {
        RedisValue::Nil => "(nil)".to_string(),
        RedisValue::Text(s) => s.clone(),
        RedisValue::Int(i) => format!("(integer) {i}"),
        RedisValue::Float(f) => format!("(double) {f}"),
        RedisValue::Bool(b) => format!("(boolean) {b}"),
        RedisValue::Bytes(b) => format!("(binary) [{} bytes]", b.len()),
        RedisValue::List(items) | RedisValue::Set(items) | RedisValue::Array(items) => items
            .iter()
            .enumerate()
            .map(|(i, x)| format!("{}) {}", i + 1, x.display_preview(256)))
            .collect::<Vec<_>>()
            .join("\n"),
        RedisValue::Hash(pairs) => pairs
            .iter()
            .map(|(k, v)| format!("{k}: {}", v.display_preview(256)))
            .collect::<Vec<_>>()
            .join("\n"),
        RedisValue::ZSet(pairs) => pairs
            .iter()
            .map(|(m, s)| format!("{} ({s})", m.display_preview(256)))
            .collect::<Vec<_>>()
            .join("\n"),
        RedisValue::Stream(entries) => entries
            .iter()
            .map(|e| {
                let fields = e
                    .fields
                    .iter()
                    .map(|(k, v)| format!("    {k}={v}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("{}\n{fields}", e.id)
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

/// 检查是否为危险命令（按前缀匹配，不区分大小写）
fn is_dangerous(raw: &str) -> bool {
    let upper = raw.trim().to_uppercase();
    DANGEROUS_PREFIXES.iter().any(|p| upper.starts_with(p))
}

impl Render for CliPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let border = theme.border;
        let bg = theme.background;
        let secondary_bg = theme.secondary;

        // 顶部工具条
        let toolbar = h_flex()
            .w_full()
            .px(px(12.0))
            .py(px(6.0))
            .border_b_1()
            .border_color(border)
            .bg(secondary_bg)
            .gap(px(8.0))
            .items_center()
            .child(div().text_xs().text_color(muted_fg).child(format!(
                "DB {} · {} 条历史",
                self.db,
                self.history.len()
            )))
            .child(div().flex_1())
            .child(
                Button::new("cli-clear")
                    .ghost()
                    .xsmall()
                    .label("清空")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.clear_history(cx))),
            );

        // 历史块
        let mut history_view = v_flex().w_full().gap(px(10.0)).p(px(12.0));
        if self.history.is_empty() {
            history_view = history_view.child(
                div()
                    .text_sm()
                    .text_color(muted_fg)
                    .child("尚无命令；在底部输入命令并 Enter 执行（如 PING / GET foo / KEYS *）"),
            );
        } else {
            for (i, entry) in self.history.iter().enumerate() {
                history_view =
                    history_view.child(render_history_entry(i, entry, fg, muted_fg, border));
            }
        }

        // 底部输入区
        let input_row = h_flex()
            .w_full()
            .px(px(12.0))
            .py(px(8.0))
            .border_t_1()
            .border_color(border)
            .gap(px(8.0))
            .items_center()
            .child(div().text_xs().text_color(muted_fg).child("⏵"))
            .child(div().flex_1().min_w_0().child(Input::new(&self.input)))
            .child(
                Button::new("cli-send")
                    .primary()
                    .small()
                    .label("执行")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.handle_submit(window, cx)
                    })),
            );

        v_flex()
            .size_full()
            .bg(bg)
            .child(toolbar)
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(history_view),
            )
            .child(input_row)
    }
}

fn render_history_entry(
    idx: usize,
    entry: &HistoryEntry,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    border: gpui::Hsla,
) -> impl IntoElement {
    let (color, body): (gpui::Hsla, String) = match &entry.response {
        ResponseKind::Ok(s) => (fg, s.clone()),
        ResponseKind::Err(s) => (gpui::red(), s.clone()),
        ResponseKind::Pending => (muted_fg, "⏳ 等待应答...".to_string()),
    };
    let elapsed_label = if matches!(entry.response, ResponseKind::Pending) {
        String::new()
    } else {
        format!("{} ms", entry.elapsed_ms)
    };
    v_flex()
        .id(SharedString::from(format!("cli-entry-{idx}")))
        .w_full()
        .gap(px(4.0))
        .child(
            h_flex()
                .w_full()
                .gap(px(8.0))
                .text_xs()
                .text_color(muted_fg)
                .child(div().child(format!("> {}", entry.command)))
                .child(div().flex_1())
                .child(div().child(elapsed_label)),
        )
        .child(
            div()
                .w_full()
                .px(px(10.0))
                .py(px(6.0))
                .border_1()
                .border_color(border)
                .rounded(px(4.0))
                .text_sm()
                .text_color(color)
                .font_family("monospace")
                .child(body),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dangerous_detection() {
        assert!(is_dangerous("FLUSHDB"));
        assert!(is_dangerous("flushall"));
        assert!(is_dangerous("config set maxmemory 1g"));
        assert!(is_dangerous("KEYS *"));
        assert!(is_dangerous("DEBUG SLEEP 5"));
        assert!(!is_dangerous("GET foo"));
        assert!(!is_dangerous("SET foo bar"));
    }

    #[test]
    fn format_int() {
        assert_eq!(format_response(&RedisValue::Int(42)), "(integer) 42");
        assert_eq!(format_response(&RedisValue::Nil), "(nil)");
    }

    #[test]
    fn format_array() {
        let v = RedisValue::Array(vec![
            RedisValue::Text("a".into()),
            RedisValue::Text("b".into()),
        ]);
        let s = format_response(&v);
        assert!(s.contains("1) a"));
        assert!(s.contains("2) b"));
    }
}
