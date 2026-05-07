//! 列宽估算 + 拖拽 + 单元格编辑器 + 数值列检测 + 排序比较 + Hsla.opacity 扩展

use gpui::{
    AnyElement, AppContext as _, Context, DragMoveEvent, IntoElement, SharedString, Styled, div,
    prelude::*, px,
};
use gpui_component::input::InputState;
use ramag_domain::entities::{Row, Value};

use crate::views::result_panel::ResultPanel;

pub(super) fn estimate_col_width(
    ci: usize,
    columns: &[String],
    column_types: &[String],
    rows: &[Row],
) -> gpui::Pixels {
    const MIN_W: f32 = 100.0;
    const MAX_W: f32 = 380.0;
    const PER_CHAR: f32 = 7.5;
    const PADDING: f32 = 28.0;

    let col_chars = columns.get(ci).map(|s| s.chars().count()).unwrap_or(0);
    // 类型副标：列名 + gap(≈1 字符) + 类型字符；保证字段名永不被截断
    let type_chars = column_types
        .get(ci)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().count() + 1)
        .unwrap_or(0);
    let header_chars = col_chars + type_chars;

    let mut max_chars = header_chars;
    // display_preview(60) 与渲染保持一致：被截断成 60 的内容自然不会撑爆 380 上限
    for row in rows.iter().take(100) {
        if let Some(v) = row.values.get(ci) {
            let chars = v.display_preview(60).chars().count();
            if chars > max_chars {
                max_chars = chars;
            }
        }
    }
    let est = max_chars as f32 * PER_CHAR + PADDING;
    px(est.clamp(MIN_W, MAX_W))
}

/// 列宽拖拽 drag value：携带列索引（被拖动的列）
#[derive(Clone)]
pub(super) struct ColResizeDrag(#[allow(dead_code)] pub usize);

impl gpui::Render for ColResizeDrag {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

/// 表头每列右边缘的拖拽 handle（4px 宽，cursor-col-resize）
pub(super) fn render_col_resize_handle(ci: usize, cx: &mut Context<ResultPanel>) -> AnyElement {
    div()
        .id(SharedString::from(format!("col-resize-{ci}")))
        .absolute()
        .right_0()
        .top_0()
        .h(px(34.0))
        .w(px(4.0))
        .cursor_col_resize()
        .on_drag(ColResizeDrag(ci), |drag, _pos, _, cx| {
            cx.new(|_| drag.clone())
        })
        .on_drag_move(
            cx.listener(move |this, e: &DragMoveEvent<ColResizeDrag>, _, cx| {
                let drag = e.drag(cx);
                let mouse_x = e.event.position.x;
                let handle_right = e.bounds.right();
                let delta = mouse_x - handle_right;
                if delta == px(0.0) {
                    return;
                }
                let cur = this.col_width_override(drag.0).unwrap_or_else(|| px(180.0));
                let new_w = (cur + delta).max(px(60.0)).min(px(800.0));
                this.set_col_width_override(drag.0, new_w);
                cx.notify();
            }),
        )
        .into_any_element()
}

pub(super) fn detect_numeric_column(ci: usize, rows: &[Row]) -> bool {
    let mut has_num = false;
    let mut all_num = true;
    for row in rows.iter().take(20) {
        if let Some(v) = row.values.get(ci) {
            match v {
                Value::Null => {}
                Value::Int(_) | Value::Float(_) => has_num = true,
                _ => {
                    all_num = false;
                    break;
                }
            }
        }
    }
    has_num && all_num
}

/// 同步打开单元格编辑弹框：必须在 listener 内调（已持 ResultPanel mut ref）
pub(super) fn open_cell_editor(
    panel: &mut ResultPanel,
    ri: usize,
    ci: usize,
    window: &mut gpui::Window,
    cx: &mut Context<ResultPanel>,
) {
    let Some((col_name, initial_text, has_pk)) = panel.cell_info(ri, ci) else {
        return;
    };
    // 视图作为只读：弹框正常打开（用于查看完整内容），但里头「确认」按钮被禁用
    let is_view = panel.target_is_view();
    let input = cx.new(|cx_inner| {
        InputState::new(window, cx_inner)
            .multi_line(true)
            .rows(8)
            .default_value(initial_text)
    });
    panel.set_cell_edit_input(Some(input.clone()));
    let panel_entity = cx.entity();
    crate::views::cell_edit_dialog::open(
        panel_entity,
        ri,
        ci,
        col_name,
        input,
        has_pk,
        is_view,
        window,
        cx,
    );
}

/// 比较两个 Value：Null 视为最小，同型按值比较，跨型用字符串兜底
pub(super) fn compare_values(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, _) => Ordering::Less,
        (_, None) => Ordering::Greater,
        (Some(x), Some(y)) => match (x, y) {
            (Value::Null, Value::Null) => Ordering::Equal,
            (Value::Null, _) => Ordering::Less,
            (_, Value::Null) => Ordering::Greater,
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            (Value::Int(a), Value::Int(b)) => a.cmp(b),
            (Value::Float(a), Value::Float(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
            (Value::Int(a), Value::Float(b)) => {
                (*a as f64).partial_cmp(b).unwrap_or(Ordering::Equal)
            }
            (Value::Float(a), Value::Int(b)) => {
                a.partial_cmp(&(*b as f64)).unwrap_or(Ordering::Equal)
            }
            (Value::Text(a), Value::Text(b)) => a.cmp(b),
            (Value::DateTime(a), Value::DateTime(b)) => a.cmp(b),
            (Value::Bytes(a), Value::Bytes(b)) => a.cmp(b),
            _ => x
                .display_preview(usize::MAX)
                .cmp(&y.display_preview(usize::MAX)),
        },
    }
}

/// Hsla 透明度便捷调用
pub(super) trait OpacityExt {
    fn opacity(self, alpha: f32) -> Self;
}

impl OpacityExt for gpui::Hsla {
    fn opacity(mut self, alpha: f32) -> Self {
        self.a = alpha.clamp(0.0, 1.0);
        self
    }
}
