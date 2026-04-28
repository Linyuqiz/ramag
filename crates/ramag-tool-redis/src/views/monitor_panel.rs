//! 监控面板：INFO 全景 + 9 大分组（server/clients/memory/persistence/stats/
//! replication/cpu/commandstats/keyspace）
//!
//! Stage 19.1 范围：
//! - 解析 INFO 全文，按 `# Section` 分组
//! - 每分组独立子面板渲染 key=value
//! - 顶部刷新按钮（手动）

use std::sync::Arc;

use gpui::{
    ClickEvent, Context, IntoElement, ParentElement, Render, SharedString, Styled, Window, div,
    prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement as _,
    v_flex,
};
use ramag_app::RedisService;
use ramag_domain::entities::{ConnectionConfig, RedisType, RedisValue};
use tracing::{error, info};

/// BigKey 扫描每条结果
#[derive(Debug, Clone)]
pub struct BigKeyResult {
    pub key: String,
    pub key_type: RedisType,
    pub size_bytes: u64,
}

#[derive(Debug, Clone)]
struct InfoSection {
    name: String,
    pairs: Vec<(String, String)>,
}

/// 慢日志单条
#[derive(Debug, Clone)]
struct SlowEntry {
    id: i64,
    timestamp: i64,
    duration_us: i64,
    command: String,
}

/// 监控视图模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MonitorView {
    InfoSection(usize),
    SlowLog,
    Latency,
    BigKey,
    Clients,
}

/// CLIENT LIST 单条客户端信息
#[derive(Debug, Clone, Default)]
struct ClientInfo {
    id: u64,
    addr: String,
    name: String,
    age_sec: u64,
    idle_sec: u64,
    cmd: String,
    db: u32,
}

/// 趋势图采样点
#[derive(Debug, Clone, Copy)]
struct MetricSample {
    /// used_memory（字节）
    memory_bytes: u64,
    /// instantaneous_ops_per_sec
    qps: u64,
}

/// 采样间隔（5s）+ 缓冲点数（60 个 = 5 分钟窗口）
const SAMPLE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
const SAMPLE_CAP: usize = 60;

pub struct MonitorPanel {
    service: Arc<RedisService>,
    config: ConnectionConfig,
    sections: Vec<InfoSection>,
    slowlog: Vec<SlowEntry>,
    latency_text: String,
    /// BigKey 扫描结果（按 size_bytes 倒序）
    bigkey_results: Vec<BigKeyResult>,
    /// BigKey 扫描进度（已扫描，总键数；总键数初次未知用 None）
    bigkey_progress: Option<(usize, Option<usize>)>,
    /// 扫描中标志（防止重复触发）
    bigkey_scanning: bool,
    /// CLIENT LIST 应答缓存
    clients: Vec<ClientInfo>,
    /// 内存 + QPS 采样历史（环形缓冲，cap = SAMPLE_CAP）
    metrics: std::collections::VecDeque<MetricSample>,
    /// 采样后台任务句柄（drop = 停止采样）
    _metrics_task: Option<gpui::Task<()>>,
    view: MonitorView,
    loading: bool,
    error: Option<String>,
}

impl MonitorPanel {
    pub fn new(service: Arc<RedisService>, config: ConnectionConfig) -> Self {
        Self {
            service,
            config,
            sections: Vec::new(),
            slowlog: Vec::new(),
            latency_text: String::new(),
            bigkey_results: Vec::new(),
            bigkey_progress: None,
            bigkey_scanning: false,
            clients: Vec::new(),
            metrics: std::collections::VecDeque::with_capacity(SAMPLE_CAP),
            _metrics_task: None,
            view: MonitorView::InfoSection(0),
            loading: false,
            error: None,
        }
    }

    /// 启动采样任务（在 cx.new 之后由外部调用，因为需要 cx.spawn 持有 entity weak ref）
    pub fn start_metrics_sampler(&mut self, cx: &mut Context<Self>) {
        if self._metrics_task.is_some() {
            return;
        }
        let svc = self.service.clone();
        let config = self.config.clone();
        let task = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SAMPLE_INTERVAL).await;
                // entity drop 后退出
                if this.update(cx, |_, _| ()).is_err() {
                    break;
                }
                let info_result = svc.info(&config, &["memory", "stats"]).await;
                let sample = match info_result {
                    Ok(text) => parse_metrics(&text),
                    Err(_) => continue,
                };
                let _ = this.update(cx, |this, cx| {
                    if this.metrics.len() >= SAMPLE_CAP {
                        this.metrics.pop_front();
                    }
                    this.metrics.push_back(sample);
                    cx.notify();
                });
            }
        });
        self._metrics_task = Some(task);
    }

    /// 公开接口：外部 Session 在切到 Monitor tab 时主动 refresh 一次（拉 INFO 全文）
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let svc = self.service.clone();
        let config = self.config.clone();
        self.loading = true;
        self.error = None;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = svc.info(&config, &[]).await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(text) => {
                        info!(len = text.len(), "info loaded");
                        this.sections = parse_info(&text);
                    }
                    Err(e) => {
                        error!(error = %e, "info failed");
                        this.error = Some(format!("INFO 加载失败：{e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_section(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.sections.len() {
            self.view = MonitorView::InfoSection(idx);
            cx.notify();
        }
    }

    fn select_view(&mut self, view: MonitorView, cx: &mut Context<Self>) {
        self.view = view;
        // 切到 SlowLog / Latency 时立即拉数据；BigKey 不自动扫描（重操作，需用户主动触发）
        match view {
            MonitorView::SlowLog => self.refresh_slowlog(cx),
            MonitorView::Latency => self.refresh_latency(cx),
            MonitorView::Clients => self.refresh_clients(cx),
            MonitorView::InfoSection(_) | MonitorView::BigKey => {}
        }
        cx.notify();
    }

    /// BigKey 扫描：SCAN 全 keys（限 5000）+ 每 key MEMORY USAGE，按 size 倒序
    fn run_bigkey_scan(&mut self, cx: &mut Context<Self>) {
        if self.bigkey_scanning {
            return;
        }
        self.bigkey_scanning = true;
        self.bigkey_results.clear();
        self.bigkey_progress = Some((0, None));
        cx.notify();

        let svc = self.service.clone();
        let config = self.config.clone();
        // 默认扫描 0 号 db；后续可加 dropdown 选 db
        let db = 0u8;
        const MAX_KEYS: usize = 5_000;

        cx.spawn(async move |this, cx| {
            // Step 1：SCAN 全 keys
            let scan_result = svc.scan_all(&config, db, None, None, MAX_KEYS).await;
            let metas = match scan_result {
                Ok(m) => m,
                Err(e) => {
                    error!(error = %e, "bigkey scan_all failed");
                    let _ = this.update(cx, |this, cx| {
                        this.bigkey_scanning = false;
                        this.error = Some(format!("BigKey 扫描失败：{e}"));
                        cx.notify();
                    });
                    return;
                }
            };
            let total = metas.len();
            let _ = this.update(cx, |this, cx| {
                this.bigkey_progress = Some((0, Some(total)));
                cx.notify();
            });

            // Step 2：每 key MEMORY USAGE + TYPE
            let mut results: Vec<BigKeyResult> = Vec::with_capacity(total);
            for (i, meta) in metas.iter().enumerate() {
                let argv = vec!["MEMORY".into(), "USAGE".into(), meta.key.clone()];
                let mem_result = svc.execute_command(&config, db, argv).await;
                let size_bytes: u64 = match mem_result {
                    Ok(RedisValue::Int(n)) if n >= 0 => n as u64,
                    _ => continue, // key 可能已过期或服务端不支持
                };
                let kind = match svc.key_type(&config, db, &meta.key).await {
                    Ok(k) => k,
                    Err(_) => RedisType::None,
                };
                results.push(BigKeyResult {
                    key: meta.key.clone(),
                    key_type: kind,
                    size_bytes,
                });
                // 每 50 个 key 更新一次进度（避免 cx.notify 风暴）
                if (i + 1) % 50 == 0 {
                    let scanned = i + 1;
                    let _ = this.update(cx, |this, cx| {
                        this.bigkey_progress = Some((scanned, Some(total)));
                        cx.notify();
                    });
                }
            }

            // Step 3：按 size_bytes 倒序排序
            results.sort_by_key(|r| std::cmp::Reverse(r.size_bytes));

            let _ = this.update(cx, |this, cx| {
                info!(count = results.len(), "bigkey scan completed");
                this.bigkey_results = results;
                this.bigkey_progress = Some((total, Some(total)));
                this.bigkey_scanning = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_slowlog(&mut self, cx: &mut Context<Self>) {
        let svc = self.service.clone();
        let config = self.config.clone();
        cx.spawn(async move |this, cx| {
            let argv = vec!["SLOWLOG".into(), "GET".into(), "100".into()];
            let result = svc.execute_command(&config, 0, argv).await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(v) => {
                    this.slowlog = parse_slowlog(&v);
                    info!(count = this.slowlog.len(), "slowlog loaded");
                    cx.notify();
                }
                Err(e) => {
                    error!(error = %e, "slowlog failed");
                    this.error = Some(format!("SLOWLOG 加载失败：{e}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn refresh_latency(&mut self, cx: &mut Context<Self>) {
        let svc = self.service.clone();
        let config = self.config.clone();
        cx.spawn(async move |this, cx| {
            let argv = vec!["LATENCY".into(), "DOCTOR".into()];
            let result = svc.execute_command(&config, 0, argv).await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(v) => {
                    this.latency_text = match v {
                        RedisValue::Text(s) => s,
                        other => other.display_preview(8192),
                    };
                    info!(len = this.latency_text.len(), "latency doctor loaded");
                    cx.notify();
                }
                Err(e) => {
                    error!(error = %e, "latency doctor failed");
                    this.error = Some(format!("LATENCY 加载失败：{e}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn refresh_clients(&mut self, cx: &mut Context<Self>) {
        let svc = self.service.clone();
        let config = self.config.clone();
        cx.spawn(async move |this, cx| {
            let argv = vec!["CLIENT".into(), "LIST".into()];
            let result = svc.execute_command(&config, 0, argv).await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(RedisValue::Text(text)) => {
                    this.clients = parse_clients(&text);
                    info!(count = this.clients.len(), "client list loaded");
                    cx.notify();
                }
                Ok(RedisValue::Bytes(b)) => {
                    let text = String::from_utf8_lossy(&b).into_owned();
                    this.clients = parse_clients(&text);
                    cx.notify();
                }
                Ok(other) => {
                    error!(?other, "client list unexpected response");
                    this.error = Some("CLIENT LIST 应答异常".into());
                    cx.notify();
                }
                Err(e) => {
                    error!(error = %e, "client list failed");
                    this.error = Some(format!("CLIENT LIST 失败：{e}"));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn kill_client(&mut self, id: u64, cx: &mut Context<Self>) {
        let svc = self.service.clone();
        let config = self.config.clone();
        cx.spawn(async move |this, cx| {
            let argv = vec!["CLIENT".into(), "KILL".into(), "ID".into(), id.to_string()];
            let _ = svc.execute_command(&config, 0, argv).await;
            // 杀完重新拉一次列表
            let _ = this.update(cx, |this, cx| {
                this.clients.retain(|c| c.id != id);
                info!(id, "client killed");
                cx.notify();
                this.refresh_clients(cx);
            });
        })
        .detach();
    }

    fn reset_slowlog(&mut self, cx: &mut Context<Self>) {
        let svc = self.service.clone();
        let config = self.config.clone();
        cx.spawn(async move |this, cx| {
            let argv = vec!["SLOWLOG".into(), "RESET".into()];
            let _ = svc.execute_command(&config, 0, argv).await;
            let _ = this.update(cx, |this, cx| {
                this.slowlog.clear();
                cx.notify();
            });
        })
        .detach();
    }
}

/// 从 INFO 全文（含 memory + stats sections）提取 used_memory + instantaneous_ops_per_sec
fn parse_metrics(info_text: &str) -> MetricSample {
    let mut memory_bytes: u64 = 0;
    let mut qps: u64 = 0;
    for line in info_text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix("used_memory:") {
            memory_bytes = rest.trim().parse().unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("instantaneous_ops_per_sec:") {
            qps = rest.trim().parse().unwrap_or(0);
        }
    }
    MetricSample { memory_bytes, qps }
}

/// CLIENT LIST 应答（多行 `key=value` 格式）→ ClientInfo 列表
fn parse_clients(text: &str) -> Vec<ClientInfo> {
    text.lines()
        .filter_map(|line| {
            let mut info = ClientInfo::default();
            let mut has_id = false;
            for kv in line.split_whitespace() {
                if let Some((k, v)) = kv.split_once('=') {
                    match k {
                        "id" => {
                            info.id = v.parse().unwrap_or(0);
                            has_id = info.id > 0;
                        }
                        "addr" => info.addr = v.to_string(),
                        "name" => info.name = v.to_string(),
                        "age" => info.age_sec = v.parse().unwrap_or(0),
                        "idle" => info.idle_sec = v.parse().unwrap_or(0),
                        "cmd" => info.cmd = v.to_string(),
                        "db" => info.db = v.parse().unwrap_or(0),
                        _ => {}
                    }
                }
            }
            if has_id { Some(info) } else { None }
        })
        .collect()
}

/// SLOWLOG GET 应答 → SlowEntry 列表
/// 应答格式：Array of Array[id, timestamp, duration_us, command_array, [client_addr, client_name]]
fn parse_slowlog(v: &RedisValue) -> Vec<SlowEntry> {
    let entries = match v {
        RedisValue::Array(a) => a,
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in entries {
        let parts = match entry {
            RedisValue::Array(p) => p,
            _ => continue,
        };
        if parts.len() < 4 {
            continue;
        }
        let id = match &parts[0] {
            RedisValue::Int(i) => *i,
            _ => 0,
        };
        let ts = match &parts[1] {
            RedisValue::Int(i) => *i,
            _ => 0,
        };
        let dur = match &parts[2] {
            RedisValue::Int(i) => *i,
            _ => 0,
        };
        let cmd_parts = match &parts[3] {
            RedisValue::Array(c) => c
                .iter()
                .map(|x| x.display_preview(128))
                .collect::<Vec<_>>()
                .join(" "),
            other => other.display_preview(256),
        };
        out.push(SlowEntry {
            id,
            timestamp: ts,
            duration_us: dur,
            command: cmd_parts,
        });
    }
    out
}

impl Render for MonitorPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 提前把 theme 内需要的颜色拷出来，让 theme 借用尽快释放
        // 否则 cx.listener 后续要 &mut cx 会与 &theme 冲突
        let (muted_fg, fg, border, bg, secondary_bg, accent) = {
            let theme = cx.theme();
            (
                theme.muted_foreground,
                theme.foreground,
                theme.border,
                theme.background,
                theme.secondary,
                theme.accent,
            )
        };
        let mut accent_tint = accent;
        accent_tint.a = 0.15;

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
            .child(div().text_xs().text_color(muted_fg).child(if self.loading {
                "加载中...".to_string()
            } else if let Some(ref e) = self.error {
                e.clone()
            } else {
                format!("{} 个分组", self.sections.len())
            }))
            .child(div().flex_1())
            .child(
                Button::new("monitor-refresh")
                    .ghost()
                    .xsmall()
                    .icon(ramag_ui::icons::refresh_cw())
                    .label("刷新")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.refresh(cx))),
            );

        // 左侧 sidebar：INFO 分组 + 分隔 + 工具入口（SlowLog / Latency）
        let mut section_list = v_flex()
            .w(px(180.0))
            .flex_none()
            .border_r_1()
            .border_color(border);
        // INFO 分组
        for (i, sec) in self.sections.iter().enumerate() {
            let is_active = matches!(self.view, MonitorView::InfoSection(idx) if idx == i);
            let id = SharedString::from(format!("info-sec-{i}"));
            let mut item = div()
                .id(id)
                .w_full()
                .px(px(12.0))
                .py(px(8.0))
                .text_sm()
                .child(sec.name.clone());
            if is_active {
                item = item.bg(accent_tint).text_color(accent);
            } else {
                item = item
                    .text_color(fg)
                    .cursor_pointer()
                    .hover(|this| this.opacity(0.75))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.select_section(i, cx);
                    }));
            }
            section_list = section_list.child(item);
        }
        // 分隔
        section_list = section_list
            .child(div().h(px(1.0)).w_full().bg(border).my(px(6.0)))
            .child(
                div()
                    .px(px(12.0))
                    .py(px(4.0))
                    .text_xs()
                    .text_color(muted_fg)
                    .child("诊断工具"),
            );
        // 工具项
        for (label, view_kind, key_id) in [
            ("⏱ 慢日志 SLOWLOG", MonitorView::SlowLog, "tool-slowlog"),
            ("📈 LATENCY DOCTOR", MonitorView::Latency, "tool-latency"),
            ("🔍 BigKey 扫描", MonitorView::BigKey, "tool-bigkey"),
            ("👥 CLIENT LIST", MonitorView::Clients, "tool-clients"),
        ] {
            let is_active = self.view == view_kind;
            let id = SharedString::from(key_id);
            let mut item = div()
                .id(id)
                .w_full()
                .px(px(12.0))
                .py(px(8.0))
                .text_sm()
                .child(label);
            if is_active {
                item = item.bg(accent_tint).text_color(accent);
            } else {
                item = item
                    .text_color(fg)
                    .cursor_pointer()
                    .hover(|this| this.opacity(0.75))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.select_view(view_kind, cx);
                    }));
            }
            section_list = section_list.child(item);
        }

        // 右侧详情按 view 分发
        let detail: gpui::AnyElement = match self.view {
            MonitorView::InfoSection(idx) => match self.sections.get(idx) {
                Some(sec) => render_info_section(sec, fg, muted_fg, border).into_any_element(),
                None => empty_hint(muted_fg).into_any_element(),
            },
            MonitorView::SlowLog => self
                .render_slowlog_view(fg, muted_fg, border, cx)
                .into_any_element(),
            MonitorView::Latency => self
                .render_latency_view(fg, muted_fg, border, cx)
                .into_any_element(),
            MonitorView::BigKey => self
                .render_bigkey_view(fg, muted_fg, border, cx)
                .into_any_element(),
            MonitorView::Clients => self
                .render_clients_view(fg, muted_fg, accent, border, cx)
                .into_any_element(),
        };

        // 趋势图（迷你版）：内存柱 + QPS 柱并排
        let chart_row = render_metrics_chart(&self.metrics, accent, muted_fg, border);

        v_flex()
            .size_full()
            .bg(bg)
            .child(toolbar)
            .child(chart_row)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(v_flex().h_full().overflow_y_scrollbar().child(section_list))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_y_scrollbar()
                            .child(detail),
                    ),
            )
    }
}

/// 趋势图行：左侧"内存"标题 + 柱状条；右侧"QPS"标题 + 柱状条
fn render_metrics_chart(
    samples: &std::collections::VecDeque<MetricSample>,
    accent: gpui::Hsla,
    muted_fg: gpui::Hsla,
    border: gpui::Hsla,
) -> impl IntoElement {
    let max_mem = samples
        .iter()
        .map(|s| s.memory_bytes)
        .max()
        .unwrap_or(1)
        .max(1);
    let max_qps = samples.iter().map(|s| s.qps).max().unwrap_or(1).max(1);
    let chart_h = 36.0_f32;

    let mem_bars = render_bar_chart(
        samples.iter().map(|s| s.memory_bytes),
        max_mem,
        chart_h,
        accent,
    );
    let qps_bars = render_bar_chart(samples.iter().map(|s| s.qps), max_qps, chart_h, accent);

    let last_mem = samples.back().map(|s| s.memory_bytes).unwrap_or(0);
    let last_qps = samples.back().map(|s| s.qps).unwrap_or(0);

    h_flex()
        .w_full()
        .px(px(12.0))
        .py(px(8.0))
        .border_b_1()
        .border_color(border)
        .gap(px(20.0))
        .items_end()
        .child(
            v_flex()
                .flex_1()
                .gap(px(2.0))
                .child(
                    h_flex()
                        .gap(px(8.0))
                        .text_xs()
                        .text_color(muted_fg)
                        .child(div().child("📈 内存"))
                        .child(div().child(format!(
                            "{} ({} B)",
                            human_readable_bytes(last_mem),
                            last_mem
                        ))),
                )
                .child(mem_bars),
        )
        .child(
            v_flex()
                .flex_1()
                .gap(px(2.0))
                .child(
                    h_flex()
                        .gap(px(8.0))
                        .text_xs()
                        .text_color(muted_fg)
                        .child(div().child("⚡ QPS"))
                        .child(div().child(format!("{last_qps} ops/s"))),
                )
                .child(qps_bars),
        )
}

/// 简易柱状图：每个采样点画一个 2px 宽的竖柱，高度按归一化值缩放到 chart_h
fn render_bar_chart(
    values: impl Iterator<Item = u64>,
    max: u64,
    chart_h: f32,
    color: gpui::Hsla,
) -> impl IntoElement {
    let mut row = h_flex().h(px(chart_h)).items_end().gap(px(1.0));
    for v in values {
        let h_ratio = (v as f32 / max as f32).clamp(0.0, 1.0);
        let h = px((chart_h * h_ratio).max(1.0));
        row = row.child(div().w(px(2.0)).h(h).bg(color));
    }
    row
}

// ===== 渲染辅助函数 =====

/// 人类可读字节数（与 key_detail 同款；此处独立一份避免跨模块 pub）
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

fn empty_hint(muted_fg: gpui::Hsla) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_sm()
                .text_color(muted_fg)
                .child("点击「刷新」加载数据"),
        )
}

fn render_info_section(
    sec: &InfoSection,
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
    for (k, v) in &sec.pairs {
        rows = rows.child(
            h_flex()
                .w_full()
                .px(px(10.0))
                .py(px(5.0))
                .border_b_1()
                .border_color(border)
                .gap(px(8.0))
                .child(
                    div()
                        .w(px(220.0))
                        .flex_none()
                        .text_xs()
                        .text_color(muted_fg)
                        .child(k.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(fg)
                        .font_family("monospace")
                        .child(v.clone()),
                ),
        );
    }
    v_flex()
        .flex_1()
        .min_w_0()
        .p(px(14.0))
        .child(
            div()
                .pb(px(10.0))
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(format!("# {}", sec.name)),
        )
        .child(rows)
}

impl MonitorPanel {
    fn render_slowlog_view(
        &self,
        fg: gpui::Hsla,
        muted_fg: gpui::Hsla,
        border: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mut rows = v_flex().w_full().gap(px(0.0));
        if self.slowlog.is_empty() {
            rows = rows.child(
                div()
                    .py(px(20.0))
                    .text_center()
                    .text_sm()
                    .text_color(muted_fg)
                    .child("（无慢命令记录）"),
            );
        } else {
            for e in &self.slowlog {
                let dur_ms = e.duration_us as f64 / 1000.0;
                rows = rows.child(
                    v_flex()
                        .w_full()
                        .px(px(10.0))
                        .py(px(8.0))
                        .border_b_1()
                        .border_color(border)
                        .gap(px(4.0))
                        .child(
                            h_flex()
                                .w_full()
                                .gap(px(12.0))
                                .text_xs()
                                .text_color(muted_fg)
                                .child(div().child(format!("#{}", e.id)))
                                .child(div().child(format!("ts {}", e.timestamp)))
                                .child(div().child(format!("耗时 {dur_ms:.3} ms"))),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(fg)
                                .font_family("monospace")
                                .child(e.command.clone()),
                        ),
                );
            }
        }

        v_flex()
            .flex_1()
            .min_w_0()
            .p(px(14.0))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap(px(8.0))
                    .pb(px(10.0))
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!("⏱ 慢日志（{}）", self.slowlog.len())),
                    )
                    .child(
                        Button::new("slowlog-refresh")
                            .ghost()
                            .xsmall()
                            .label("刷新")
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.refresh_slowlog(cx)),
                            ),
                    )
                    .child(
                        Button::new("slowlog-reset")
                            .danger()
                            .xsmall()
                            .label("清空")
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.reset_slowlog(cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .border_1()
                    .border_color(border)
                    .rounded(px(4.0))
                    .child(rows),
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_clients_view(
        &self,
        fg: gpui::Hsla,
        muted_fg: gpui::Hsla,
        accent: gpui::Hsla,
        border: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let mut rows = v_flex()
            .w_full()
            .gap(px(0.0))
            .border_1()
            .border_color(border)
            .rounded(px(4.0));
        if self.clients.is_empty() {
            rows = rows.child(
                div()
                    .py(px(20.0))
                    .text_center()
                    .text_sm()
                    .text_color(muted_fg)
                    .child("（无客户端 / 点刷新）"),
            );
        } else {
            // 头部
            rows = rows.child(
                h_flex()
                    .w_full()
                    .px(px(10.0))
                    .py(px(6.0))
                    .border_b_1()
                    .border_color(border)
                    .gap(px(8.0))
                    .text_xs()
                    .text_color(muted_fg)
                    .child(div().w(px(60.0)).child("ID"))
                    .child(div().w(px(180.0)).child("Addr"))
                    .child(div().w(px(120.0)).child("Name"))
                    .child(div().w(px(60.0)).child("DB"))
                    .child(div().w(px(80.0)).child("Age"))
                    .child(div().w(px(80.0)).child("Idle"))
                    .child(div().flex_1().min_w_0().child("Cmd"))
                    .child(div().w(px(50.0))),
            );
            for c in &self.clients {
                let id_for_kill = c.id;
                let kill_id = SharedString::from(format!("client-kill-{}", c.id));
                rows = rows.child(
                    h_flex()
                        .w_full()
                        .px(px(10.0))
                        .py(px(6.0))
                        .border_b_1()
                        .border_color(border)
                        .gap(px(8.0))
                        .text_sm()
                        .text_color(fg)
                        .font_family("monospace")
                        .child(div().w(px(60.0)).child(c.id.to_string()))
                        .child(
                            div()
                                .w(px(180.0))
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(c.addr.clone()),
                        )
                        .child(div().w(px(120.0)).overflow_hidden().text_ellipsis().child(
                            if c.name.is_empty() {
                                "—".to_string()
                            } else {
                                c.name.clone()
                            },
                        ))
                        .child(div().w(px(60.0)).child(c.db.to_string()))
                        .child(div().w(px(80.0)).child(format!("{}s", c.age_sec)))
                        .child(div().w(px(80.0)).child(format!("{}s", c.idle_sec)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(c.cmd.clone()),
                        )
                        .child(
                            div()
                                .id(kill_id)
                                .w(px(50.0))
                                .text_xs()
                                .text_color(accent)
                                .cursor_pointer()
                                .hover(|this| this.opacity(0.7))
                                .child("断开")
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.kill_client(id_for_kill, cx);
                                })),
                        ),
                );
            }
        }

        v_flex()
            .flex_1()
            .min_w_0()
            .p(px(14.0))
            .gap(px(10.0))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!("👥 客户端连接（{}）", self.clients.len())),
                    )
                    .child(
                        Button::new("clients-refresh")
                            .ghost()
                            .xsmall()
                            .label("刷新")
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.refresh_clients(cx)),
                            ),
                    ),
            )
            .child(rows)
    }

    fn render_bigkey_view(
        &self,
        fg: gpui::Hsla,
        muted_fg: gpui::Hsla,
        border: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let progress_text = match self.bigkey_progress {
            Some((scanned, Some(total))) => format!("{scanned} / {total}"),
            Some((scanned, None)) => format!("已扫描 {scanned}"),
            None => "未开始".to_string(),
        };

        // 按类型分组取 top 10
        let mut by_type: std::collections::BTreeMap<&'static str, Vec<&BigKeyResult>> =
            Default::default();
        for r in &self.bigkey_results {
            by_type.entry(r.key_type.label()).or_default().push(r);
        }
        // 每组限 top 10
        for v in by_type.values_mut() {
            v.truncate(10);
        }

        let mut groups_view = v_flex().w_full().gap(px(14.0));
        if !self.bigkey_results.is_empty() {
            for (type_label, items) in &by_type {
                let mut rows = v_flex()
                    .w_full()
                    .gap(px(0.0))
                    .border_1()
                    .border_color(border)
                    .rounded(px(4.0));
                for r in items {
                    let size_text = format!(
                        "{}（{} B）",
                        human_readable_bytes(r.size_bytes),
                        r.size_bytes
                    );
                    rows = rows.child(
                        h_flex()
                            .w_full()
                            .px(px(10.0))
                            .py(px(6.0))
                            .border_b_1()
                            .border_color(border)
                            .gap(px(8.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .text_color(fg)
                                    .font_family("monospace")
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(r.key.clone()),
                            )
                            .child(div().text_xs().text_color(muted_fg).child(size_text)),
                    );
                }
                groups_view = groups_view.child(
                    v_flex()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(format!("📦 {type_label}（top {} by size）", items.len())),
                        )
                        .child(rows),
                );
            }
        }

        v_flex()
            .flex_1()
            .min_w_0()
            .p(px(14.0))
            .gap(px(12.0))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!("🔍 BigKey 扫描（{progress_text}）")),
                    )
                    .child(
                        Button::new("bigkey-scan")
                            .primary()
                            .small()
                            .label(if self.bigkey_scanning {
                                "扫描中..."
                            } else if self.bigkey_results.is_empty() {
                                "开始扫描"
                            } else {
                                "重新扫描"
                            })
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.run_bigkey_scan(cx)),
                            ),
                    ),
            )
            .child(div().text_xs().text_color(muted_fg).child(
                "扫描方式：SCAN 全 keys（限 5000）+ 每 key 调 MEMORY USAGE，按类型分组 top 10",
            ))
            .child(groups_view)
    }

    fn render_latency_view(
        &self,
        fg: gpui::Hsla,
        muted_fg: gpui::Hsla,
        border: gpui::Hsla,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        v_flex()
            .flex_1()
            .min_w_0()
            .p(px(14.0))
            .gap(px(10.0))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("📈 LATENCY DOCTOR"),
                    )
                    .child(
                        Button::new("latency-refresh")
                            .ghost()
                            .xsmall()
                            .label("刷新")
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.refresh_latency(cx)),
                            ),
                    ),
            )
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
                    .child(if self.latency_text.is_empty() {
                        "(尚无数据，点击刷新)".to_string()
                    } else {
                        self.latency_text.clone()
                    }),
            )
            .child(
                div().text_xs().text_color(muted_fg).child(
                    "提示：LATENCY DOCTOR 需要服务端启用 latency-monitor-threshold（默认关闭）",
                ),
            )
    }
}

/// 解析 Redis INFO 应答文本为分组列表
///
/// 格式：
/// ```text
/// # Server
/// redis_version:7.2.4
/// redis_mode:standalone
///
/// # Clients
/// connected_clients:5
/// ```
fn parse_info(text: &str) -> Vec<InfoSection> {
    let mut sections = Vec::new();
    let mut current: Option<InfoSection> = None;
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("# ") {
            // 切到新 section
            if let Some(s) = current.take() {
                sections.push(s);
            }
            current = Some(InfoSection {
                name: rest.trim().to_string(),
                pairs: Vec::new(),
            });
        } else if let Some(sec) = current.as_mut()
            && let Some((k, v)) = line.split_once(':')
        {
            sec.pairs.push((k.to_string(), v.to_string()));
        }
    }
    if let Some(s) = current {
        sections.push(s);
    }
    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_info_basic() {
        let text = "# Server\r\nredis_version:7.2.4\r\nredis_mode:standalone\r\n\r\n# Clients\r\nconnected_clients:5\r\n";
        let secs = parse_info(text);
        assert_eq!(secs.len(), 2);
        assert_eq!(secs[0].name, "Server");
        assert_eq!(secs[0].pairs.len(), 2);
        assert_eq!(secs[0].pairs[0], ("redis_version".into(), "7.2.4".into()));
        assert_eq!(secs[1].name, "Clients");
        assert_eq!(secs[1].pairs[0].1, "5");
    }

    #[test]
    fn parse_info_empty() {
        let secs = parse_info("");
        assert!(secs.is_empty());
    }

    #[test]
    fn parse_info_value_with_colon() {
        // commandstats 行格式：cmdstat_set:calls=10,usec=20
        let text = "# Commandstats\r\ncmdstat_set:calls=10,usec=20\r\n";
        let secs = parse_info(text);
        assert_eq!(secs.len(), 1);
        assert_eq!(
            secs[0].pairs[0],
            ("cmdstat_set".into(), "calls=10,usec=20".into())
        );
    }
}
