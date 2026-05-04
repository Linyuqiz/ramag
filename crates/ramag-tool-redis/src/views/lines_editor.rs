//! List / Set 共用的单列行编辑器
//!
//! - `LinesKind::List`：行号 + 插入方向（默认尾部 RPUSH）
//! - `LinesKind::Set`：无行号；提交时主对话框做去重

use gpui::{
    App, ClickEvent, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    v_flex,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinesKind {
    List,
    Set,
}

/// List 插入方向（Set 忽略此字段）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushDir {
    Head,
    Tail,
}

struct LineRow {
    id: u64,
    input: Entity<InputState>,
}

pub struct LinesEditor {
    kind: LinesKind,
    rows: Vec<LineRow>,
    next_id: u64,
    push_dir: PushDir,
}

impl LinesEditor {
    pub fn new(kind: LinesKind, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut me = Self {
            kind,
            rows: Vec::new(),
            next_id: 0,
            push_dir: PushDir::Tail,
        };
        // 默认起始 1 行，避免空表单看起来不知所措
        me.add_row(window, cx);
        me
    }

    pub fn push_dir(&self) -> PushDir {
        self.push_dir
    }

    fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = match self.kind {
            LinesKind::List => "元素值",
            LinesKind::Set => "成员",
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let id = self.next_id;
        self.next_id += 1;
        self.rows.push(LineRow { id, input });
        cx.notify();
    }

    fn remove_row(&mut self, id: u64, cx: &mut Context<Self>) {
        // 至少留 1 行：首行不渲染删除按钮，此处保险再判一次
        if self.rows.len() <= 1 {
            return;
        }
        self.rows.retain(|r| r.id != id);
        cx.notify();
    }

    fn set_dir(&mut self, dir: PushDir, cx: &mut Context<Self>) {
        if self.push_dir != dir {
            self.push_dir = dir;
            cx.notify();
        }
    }

    /// 收集所有非空行（trim 末尾 \r）
    pub fn collect(&self, cx: &App) -> Vec<String> {
        self.rows
            .iter()
            .filter_map(|r| {
                let raw = r.input.read(cx).value().to_string();
                let trimmed = raw.trim_end_matches('\r').to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            })
            .collect()
    }
}

impl Render for LinesEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let accent = theme.accent;
        let fg = theme.foreground;
        let secondary_bg = theme.secondary;
        let border = theme.border;

        let mut accent_tint = accent;
        accent_tint.a = 0.10;
        let mut accent_border = accent;
        accent_border.a = 0.55;

        // === toolbar：[+ 添加] + 计数 + （List）方向选择 ===
        let label_add = match self.kind {
            LinesKind::List => "添加元素",
            LinesKind::Set => "添加成员",
        };
        let count_label = match self.kind {
            LinesKind::List => format!("{} 个元素", self.rows.len()),
            LinesKind::Set => format!("{} 行（提交时去重）", self.rows.len()),
        };

        let mut toolbar = h_flex()
            .w_full()
            .items_center()
            .gap(px(10.0))
            .child(
                Button::new("le-add")
                    .outline()
                    .small()
                    .icon(IconName::Plus)
                    .label(label_add)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.add_row(window, cx);
                    })),
            )
            .child(div().text_xs().text_color(muted_fg).child(count_label));

        if matches!(self.kind, LinesKind::List) {
            let dir = self.push_dir;
            let dir_chip = |this_dir: PushDir, label: &'static str, id: &'static str| {
                let is_selected = this_dir == dir;
                let mut chip = h_flex()
                    .id(SharedString::from(id))
                    .items_center()
                    .justify_center()
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded_md()
                    .border_1()
                    .text_xs()
                    .child(label.to_string());
                if is_selected {
                    chip = chip
                        .bg(accent_tint)
                        .border_color(accent_border)
                        .text_color(accent);
                } else {
                    chip = chip
                        .bg(secondary_bg)
                        .border_color(border)
                        .text_color(fg)
                        .cursor_pointer()
                        .hover(move |this| this.border_color(accent_border));
                }
                chip
            };
            toolbar = toolbar
                .child(div().flex_1())
                .child(div().text_xs().text_color(muted_fg).child("插入位置"))
                .child(
                    dir_chip(PushDir::Head, "头部 LPUSH", "le-dir-head").on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.set_dir(PushDir::Head, cx)),
                    ),
                )
                .child(
                    dir_chip(PushDir::Tail, "尾部 RPUSH", "le-dir-tail").on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.set_dir(PushDir::Tail, cx)),
                    ),
                );
        }

        // === 行列表 ===
        let mut list = v_flex().w_full().gap(px(6.0));
        for (idx, row) in self.rows.iter().enumerate() {
            let id = row.id;
            let mut line = h_flex().w_full().items_center().gap(px(8.0));
            // List 显示行号
            if matches!(self.kind, LinesKind::List) {
                line = line.child(
                    div()
                        .w(px(28.0))
                        .flex_none()
                        .text_xs()
                        .text_color(muted_fg)
                        .child(format!("{}", idx + 1)),
                );
            }
            line = line.child(div().flex_1().min_w_0().child(Input::new(&row.input)));
            // 仅在 >1 行时显示删除按钮（保留至少 1 行，无空态）
            if self.rows.len() > 1 {
                line = line.child(
                    Button::new(SharedString::from(format!("le-rm-{id}")))
                        .ghost()
                        .small()
                        .icon(IconName::Close)
                        .tooltip("删除该行")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.remove_row(id, cx);
                        })),
                );
            }
            list = list.child(line);
        }

        // toolbar 放底部：行列表在上，添加/方向/计数 chip 在下，避免左上角先看到操作按钮
        v_flex().w_full().gap(px(10.0)).child(list).child(toolbar)
    }
}
