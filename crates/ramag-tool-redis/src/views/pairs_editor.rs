//! Hash / ZSet / Stream 共用双列行编辑器。collect 时一次返回首个错误（含行号）

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
pub enum PairsKind {
    Hash,
    ZSet,
    Stream,
}

struct PairRow {
    id: u64,
    left: Entity<InputState>,
    right: Entity<InputState>,
}

pub struct PairsEditor {
    kind: PairsKind,
    rows: Vec<PairRow>,
    next_id: u64,
}

impl PairsEditor {
    pub fn new(kind: PairsKind, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut me = Self {
            kind,
            rows: Vec::new(),
            next_id: 0,
        };
        me.add_row(window, cx);
        me
    }

    fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (lph, rph) = placeholders(self.kind);
        let left = cx.new(|cx| InputState::new(window, cx).placeholder(lph));
        let right = cx.new(|cx| InputState::new(window, cx).placeholder(rph));
        let id = self.next_id;
        self.next_id += 1;
        self.rows.push(PairRow { id, left, right });
        cx.notify();
    }

    fn remove_row(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.rows.len() <= 1 {
            return;
        }
        self.rows.retain(|r| r.id != id);
        cx.notify();
    }

    /// 收集 + 行级校验
    /// - 整行（左右皆）空 → 跳过
    /// - 否则按 kind 校验 left；失败返回带行号的错误
    pub fn collect(&self, cx: &App) -> Result<Vec<(String, String)>, String> {
        let mut out = Vec::new();
        for (idx, row) in self.rows.iter().enumerate() {
            let left_raw = row.left.read(cx).value().to_string();
            let left = left_raw.trim().to_string();
            let right = row.right.read(cx).value().to_string();
            if left.is_empty() && right.is_empty() {
                continue;
            }
            match self.kind {
                PairsKind::Hash | PairsKind::Stream => {
                    if left.is_empty() {
                        return Err(format!("第 {} 行：字段名不能为空", idx + 1));
                    }
                }
                PairsKind::ZSet => {
                    if left.is_empty() {
                        return Err(format!("第 {} 行：score 不能为空", idx + 1));
                    }
                    if left.parse::<f64>().is_err() {
                        return Err(format!(
                            "第 {} 行：score 必须是数字（如 1.5），实得 `{left}`",
                            idx + 1
                        ));
                    }
                    if right.trim().is_empty() {
                        return Err(format!("第 {} 行：成员名不能为空", idx + 1));
                    }
                }
            }
            out.push((left, right));
        }
        Ok(out)
    }
}

impl Render for PairsEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;

        let (add_label, count_unit, left_width) = match self.kind {
            PairsKind::Hash => ("添加字段", "字段", 180.0_f32),
            PairsKind::ZSet => ("添加成员", "成员", 120.0_f32),
            PairsKind::Stream => ("添加字段", "字段", 180.0_f32),
        };

        let toolbar = h_flex()
            .w_full()
            .items_center()
            .gap(px(10.0))
            .child(
                Button::new("pe-add")
                    .outline()
                    .small()
                    .icon(IconName::Plus)
                    .label(add_label)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.add_row(window, cx);
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted_fg)
                    .child(format!("{} 个{count_unit}", self.rows.len())),
            );

        let mut list = v_flex().w_full().gap(px(6.0));
        for row in &self.rows {
            let id = row.id;
            let mut line = h_flex()
                .w_full()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .w(px(left_width))
                        .flex_none()
                        .child(Input::new(&row.left)),
                )
                .child(div().flex_1().min_w_0().child(Input::new(&row.right)));
            if self.rows.len() > 1 {
                line = line.child(
                    Button::new(SharedString::from(format!("pe-rm-{id}")))
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

        // toolbar 放底部：行列表在上，添加/计数 chip 在下，避免左上角先看到操作按钮
        v_flex().w_full().gap(px(10.0)).child(list).child(toolbar)
    }
}

fn placeholders(kind: PairsKind) -> (&'static str, &'static str) {
    match kind {
        PairsKind::Hash => ("字段名（如 name）", "字段值"),
        PairsKind::ZSet => ("score（数字）", "成员名"),
        PairsKind::Stream => ("字段名", "字段值"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // collect 的纯函数性质：不依赖 GPUI runtime；
    // 行级校验逻辑放主对话框单测会更顺，这里仅留 placeholder 静态校验
    #[test]
    fn placeholders_distinct_per_kind() {
        let h = placeholders(PairsKind::Hash);
        let z = placeholders(PairsKind::ZSet);
        let s = placeholders(PairsKind::Stream);
        assert_ne!(h.0, z.0);
        assert_ne!(z.0, s.0);
        assert!(z.0.contains("score"));
    }
}
