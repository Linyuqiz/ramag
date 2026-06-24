//! Redis 命令行控制台：自由输入命令 + redis-cli 风格的滚动应答历史（transcript）。
//!
//! - 输入框 Enter 执行；上方滚动显示「命令 + 应答」块流，内容保留至手动清空
//! - argv 经 format::tokenize 解析（支持引号）；应答经 format::lines_of 递归格式化
//! - 写命令在生产（只读）连接由 driver 层拦截返回 Forbidden，这里按错误行展示
//! - 显隐由 RedisSession 控制（cmd-e / 工具栏图标 / 点击外部关闭），本面板只管内容

mod complete;
mod format;

use std::sync::Arc;
use std::time::Instant;

use gpui::{
    ClickEvent, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState, MoveDown, MoveUp},
    scroll::ScrollableElement as _,
    v_flex,
};
use ramag_app::RedisService;
use ramag_domain::entities::ConnectionConfig;
use tracing::{error, info};

/// 单条命令 + 应答历史
struct Entry {
    command: String,
    db: u8,
    outcome: Outcome,
    elapsed_ms: u128,
}

enum Outcome {
    Pending,
    Ok(String),
    Err(String),
}

pub struct CliConsole {
    service: Arc<RedisService>,
    config: ConnectionConfig,
    db: u8,
    history: Vec<Entry>,
    input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl CliConsole {
    pub fn new(
        service: Arc<RedisService>,
        config: ConnectionConfig,
        db: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder("输入 Redis 命令，Enter 执行（如 GET foo）");
            // 命令名补全 + 语法提示
            state.lsp.completion_provider = Some(complete::RedisCompletionProvider::new_rc());
            state
        });
        let subs = vec![cx.subscribe_in(
            &input,
            window,
            |this: &mut Self, _, e: &InputEvent, window, cx| {
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
            _subscriptions: subs,
        }
    }

    /// 会话切 DB 时同步（应答按执行时所在 db 记录）
    pub fn set_db(&mut self, db: u8, cx: &mut Context<Self>) {
        self.db = db;
        cx.notify();
    }

    pub fn focus_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |state, cx| state.focus(window, cx));
    }

    fn handle_submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.input.read(cx).value().trim().to_string();
        if raw.is_empty() {
            return;
        }
        // 引号解析失败：就地记错误行，不发后端
        let argv = match format::tokenize(&raw) {
            Ok(a) if a.is_empty() => return,
            Ok(a) => a,
            Err(msg) => {
                self.history.push(Entry {
                    command: raw,
                    db: self.db,
                    outcome: Outcome::Err(format!("(error) 解析失败：{msg}")),
                    elapsed_ms: 0,
                });
                self.input.update(cx, |s, cx| s.set_value("", window, cx));
                cx.notify();
                return;
            }
        };

        let idx = self.history.len();
        self.history.push(Entry {
            command: raw,
            db: self.db,
            outcome: Outcome::Pending,
            elapsed_ms: 0,
        });
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
        cx.notify();

        let svc = self.service.clone();
        let config = self.config.clone();
        let db = self.db;
        let start = Instant::now();
        cx.spawn(async move |this, cx| {
            let result = svc.execute_command(&config, db, argv).await;
            let elapsed = start.elapsed().as_millis();
            let _ = this.update(cx, |this, cx| {
                if let Some(entry) = this.history.get_mut(idx) {
                    entry.elapsed_ms = elapsed;
                    entry.outcome = match result {
                        Ok(v) => {
                            info!(elapsed_ms = elapsed, "cli command ok");
                            Outcome::Ok(format::lines_of(&v).join("\n"))
                        }
                        Err(e) => {
                            error!(error = %e, "cli command failed");
                            // 仿 redis-cli：(error) + 纯消息体，不带「查询执行失败:」SQL 腔前缀
                            Outcome::Err(format!("(error) {}", e.message()))
                        }
                    };
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.history.clear();
        cx.notify();
    }
}

impl Render for CliConsole {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let border = theme.border;
        let bg = theme.background;
        let secondary_bg = theme.secondary;
        let accent = theme.primary;

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
                "命令行 · DB {} · {} 条",
                self.db,
                self.history.len()
            )))
            .child(div().flex_1())
            .child(
                Button::new("cli-clear")
                    .ghost()
                    .xsmall()
                    .icon(ramag_ui::icons::trash())
                    .tooltip("清空历史")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.clear(cx))),
            );

        let mut transcript = v_flex().w_full().gap(px(10.0)).p(px(12.0));
        if self.history.is_empty() {
            transcript = transcript.child(div().text_sm().text_color(muted_fg).child(
                "尚无命令；输入并 Enter 执行（PING / GET foo / KEYS * / CONFIG GET maxmemory）",
            ));
        } else {
            // 最新在上：刚执行的命令结果紧贴输入框下方，无需滚动
            for (i, entry) in self.history.iter().enumerate().rev() {
                transcript = transcript.child(render_entry(i, entry, fg, muted_fg, accent, border));
            }
        }

        let input_row = h_flex()
            .w_full()
            .px(px(12.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(border)
            .gap(px(8.0))
            .items_center()
            .child(div().text_xs().text_color(muted_fg).child("⏵"))
            .child(div().flex_1().min_w_0().child(Input::new(&self.input)))
            .child(
                Button::new("cli-run")
                    .primary()
                    .small()
                    .icon(IconName::Play)
                    .tooltip("执行 (Enter)")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.handle_submit(window, cx)
                    })),
            );

        // 顶部输入（补全朝下展开、最新结果就在其下）；下方 transcript 最新在上
        v_flex()
            .size_full()
            .bg(bg)
            // 单行输入不挂 up/down handler（gpui-component 限制），手动把 ↑/↓ 转发给补全菜单导航
            .on_action(cx.listener(|this, _: &MoveUp, window, cx| {
                this.input.update(cx, |state, cx| {
                    state.handle_action_for_context_menu(Box::new(MoveUp), window, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &MoveDown, window, cx| {
                this.input.update(cx, |state, cx| {
                    state.handle_action_for_context_menu(Box::new(MoveDown), window, cx);
                });
            }))
            .child(toolbar)
            .child(input_row)
            .child(
                // 外层 flex_1+min_h_0 给确定高度，内层 size_full+overflow 才能滚
                div()
                    .flex_1()
                    .min_h_0()
                    .child(div().size_full().overflow_y_scrollbar().child(transcript)),
            )
    }
}

fn render_entry(
    idx: usize,
    entry: &Entry,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    accent: gpui::Hsla,
    border: gpui::Hsla,
) -> impl IntoElement {
    // 应答区：成功按行分色（整数/浮点/布尔→强调色，nil/empty→弱化，余→前景）；错误整体红
    let body = match &entry.outcome {
        Outcome::Pending => div()
            .text_color(muted_fg)
            .child("执行中…")
            .into_any_element(),
        Outcome::Err(s) => div()
            .text_color(gpui::red())
            .child(s.clone())
            .into_any_element(),
        Outcome::Ok(s) => v_flex()
            .w_full()
            .children(s.lines().map(|line| {
                div()
                    .w_full()
                    .text_color(line_color(line, fg, muted_fg, accent))
                    .child(line.to_string())
            }))
            .into_any_element(),
    };
    let meta = if matches!(entry.outcome, Outcome::Pending) {
        format!("DB {}", entry.db)
    } else {
        format!("DB {} · {} ms", entry.db, entry.elapsed_ms)
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
                .child(
                    div()
                        .font_family("monospace")
                        .child(format!("> {}", entry.command)),
                )
                .child(div().flex_1())
                .child(div().child(meta)),
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
                .font_family("monospace")
                .child(body),
        )
}

/// 应答单行配色：按 redis-cli 类型标记粗判
fn line_color(line: &str, fg: gpui::Hsla, muted_fg: gpui::Hsla, accent: gpui::Hsla) -> gpui::Hsla {
    if line.contains("(integer)") || line.contains("(double)") || line.contains("(boolean)") {
        accent
    } else if line.contains("(nil)") || line.contains("(empty)") {
        muted_fg
    } else {
        fg
    }
}
