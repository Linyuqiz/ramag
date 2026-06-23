//! ZSet 块：双击行改 score + 删除按钮 + score 短格式

use gpui::{
    ClickEvent, Context, IntoElement, ParentElement, SharedString, Styled, div, prelude::*, px,
};
use gpui_component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use ramag_domain::entities::RedisValue;

use super::{KeyDetailEvent, KeyDetailPanel};

#[allow(clippy::too_many_arguments)]
pub(super) fn render_zset_block(
    panel: &mut Context<KeyDetailPanel>,
    key: String,
    pairs: &[(RedisValue, f64)],
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    _accent: gpui::Hsla,
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
        // 整数 score 显 "234"；小数显 "1.5"（去尾随零）；避免占满 6 位小数
        let score_str = pretty_score(*score);
        let score_for_edit = score_str.clone();
        let key_for_edit = key.clone();
        let key_for_del = key.clone();
        let raw_for_edit = raw_member.clone();
        let raw_for_del = raw_member.clone();
        let row_id = SharedString::from(format!("zset-row-{i}"));
        let del_id = SharedString::from(format!("zset-del-{i}"));
        rows = rows.child(
            h_flex()
                .id(row_id)
                .w_full()
                .px(px(8.0))
                .py(px(6.0))
                .border_b_1()
                .border_color(border)
                .gap(px(8.0))
                .cursor_pointer()
                // 双击该行打开「改 score」窗口（与其它面板一致，去掉行内编辑图标）
                .on_click(panel.listener(move |_, e: &ClickEvent, _, cx| {
                    if e.click_count() >= 2 {
                        cx.emit(KeyDetailEvent::RequestEditZSetScore(
                            key_for_edit.clone(),
                            raw_for_edit.clone(),
                            score_for_edit.clone(),
                        ));
                    }
                }))
                .child(
                    div()
                        .w(px(320.0))
                        .text_xs()
                        .text_color(muted_fg)
                        .font_family("monospace")
                        .flex_none()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(score_str.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(fg)
                        .font_family("monospace")
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(preview),
                )
                .child(
                    Button::new(del_id)
                        .ghost()
                        .small()
                        .icon(ramag_ui::icons::trash())
                        .tooltip("删除该成员")
                        .on_click(panel.listener(move |_, _: &ClickEvent, _, cx| {
                            cx.emit(KeyDetailEvent::RequestDeleteZSetMember(
                                key_for_del.clone(),
                                raw_for_del.clone(),
                            ));
                        })),
                ),
        );
    }
    rows
}

/// score 短格式：整数（i64 范围内）不带小数；其他走 Display（已去尾随零）
fn pretty_score(s: f64) -> String {
    if s.is_finite() && s == s.trunc() && s.abs() < 1e15 {
        format!("{}", s as i64)
    } else {
        format!("{s}")
    }
}

#[cfg(test)]
mod tests {
    use super::pretty_score;

    #[test]
    fn integer_no_fraction() {
        assert_eq!(pretty_score(234.0), "234");
        assert_eq!(pretty_score(0.0), "0");
        assert_eq!(pretty_score(-7.0), "-7");
    }

    #[test]
    fn float_keeps_decimal() {
        assert_eq!(pretty_score(1.5), "1.5");
        assert_eq!(pretty_score(-0.25), "-0.25");
    }

    #[test]
    fn very_large_uses_default_display() {
        // 超过 i64 安全范围的整数浮点：用默认 Display（科学计数法）
        let s = pretty_score(1e16);
        assert!(s.contains('e') || s.starts_with("10000000"), "got: {s}");
    }

    #[test]
    fn nan_uses_default_display() {
        assert_eq!(pretty_score(f64::NAN), "NaN");
    }

    #[test]
    fn infinity_uses_default_display() {
        assert_eq!(pretty_score(f64::INFINITY), "inf");
        assert_eq!(pretty_score(f64::NEG_INFINITY), "-inf");
    }
}
