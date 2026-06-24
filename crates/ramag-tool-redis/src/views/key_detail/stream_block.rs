//! Stream 块：uniform_list 行级虚拟化。条目「头行 + 各字段行」扁平成等高行序列，
//! 因 entry 字段数可变（不等高），无法直接按 entry 虚拟化，故先扁平再喂 uniform_list

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled,
    UniformListScrollHandle, div, px, uniform_list,
};
use gpui_component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};
use ramag_domain::entities::StreamEntry;

use super::{KeyDetailEvent, KeyDetailPanel};

/// 行高固定 28px：uniform_list 行级虚拟化要求等高（头行 / 字段行同高）
const ROW_H: f32 = 28.0;

/// 扁平后的行：条目头（ID + 删除按钮）或单个字段（k=v）
enum StreamRow {
    Header { id: String, idx: usize },
    Field { k: String, v: String },
}

pub(super) fn render_stream_block(
    panel: &mut Context<KeyDetailPanel>,
    key: String,
    entries: &[StreamEntry],
    scroll: &UniformListScrollHandle,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    border: gpui::Hsla,
) -> impl IntoElement + use<> {
    // 扁平化：每条 entry → 1 个头行 + N 个字段行
    let mut flat: Vec<StreamRow> = Vec::new();
    for (idx, e) in entries.iter().enumerate() {
        flat.push(StreamRow::Header {
            id: e.id.clone(),
            idx,
        });
        for (k, v) in &e.fields {
            flat.push(StreamRow::Field {
                k: k.clone(),
                v: v.clone(),
            });
        }
    }
    let rows = Rc::new(flat);
    let count = rows.len();
    let rows_for_closure = rows.clone();

    div()
        .flex_1()
        .min_h_0()
        .border_1()
        .border_color(border)
        .rounded(px(4.0))
        .child(
            uniform_list(
                "stream-rows",
                count,
                panel.processor(move |_this, range: Range<usize>, _w, cx| {
                    range
                        .filter_map(|i| {
                            let row = rows_for_closure.get(i)?;
                            Some(stream_row(row, &key, fg, muted_fg, border, cx))
                        })
                        .collect()
                }),
            )
            .track_scroll(scroll)
            .flex_1(),
        )
}

fn stream_row(
    row: &StreamRow,
    key: &str,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    border: gpui::Hsla,
    cx: &mut Context<KeyDetailPanel>,
) -> AnyElement {
    match row {
        StreamRow::Header { id, idx } => {
            let id_for_del = id.clone();
            let key_for_del = key.to_string();
            let del_id = SharedString::from(format!("stream-del-{idx}"));
            h_flex()
                .h(px(ROW_H))
                .flex_none()
                .w_full()
                .items_center()
                .gap(px(8.0))
                .px(px(8.0))
                // 顶边线分隔相邻条目
                .border_t_1()
                .border_color(border)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(fg)
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(id.clone()),
                )
                .child(
                    Button::new(del_id)
                        .ghost()
                        .xsmall()
                        .icon(ramag_ui::icons::trash())
                        .tooltip("删除该条目")
                        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(KeyDetailEvent::RequestDeleteStreamEntry(
                                key_for_del.clone(),
                                id_for_del.clone(),
                            ));
                        })),
                )
                .into_any_element()
        }
        StreamRow::Field { k, v } => h_flex()
            .h(px(ROW_H))
            .flex_none()
            .w_full()
            .items_center()
            .gap(px(8.0))
            .pl(px(20.0))
            .pr(px(8.0))
            .child(
                div()
                    .w(px(140.0))
                    .text_xs()
                    .text_color(muted_fg)
                    .flex_none()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(k.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(fg)
                    .font_family("monospace")
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(v.clone()),
            )
            .into_any_element(),
    }
}
