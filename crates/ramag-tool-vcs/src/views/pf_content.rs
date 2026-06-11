//! Project Files 主区：渲染选中文件内容（独立于 diff）。
//! 行号 + 内容两列 mono；uniform_list 处理 Y 虚拟化，外层 `overflow_x_scroll` 处理 X 横滚

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement, SharedString, Styled,
    div, prelude::*, px, uniform_list,
};
use gpui_component::{ActiveTheme, h_flex, v_flex};

use super::vcs_view::VcsView;

/// 禁用 GPUI 单轴 scroll 的"另一方向劫持"，wheel 严格按方向消费
trait RestrictScrollExt: Styled + Sized {
    fn restrict_scroll_to_axis(mut self) -> Self {
        self.style().restrict_scroll_to_axis = Some(true);
        self
    }
}
impl<T: Styled> RestrictScrollExt for T {}

/// 单行高度直接复用 diff 的行高常量：两个视图字号同为 text_xs，
/// 行高再不一致会产生「diff 字体更大」的密度错觉
use super::diff_panel::DIFF_ROW_H as LINE_HEIGHT;
/// 等宽字体单字符估算宽度（单位 px；mono 字体在 13px size 下约 7.5px/字）
const MONO_CHAR_W: f32 = 7.5;

impl VcsView {
    /// 主入口：根据 loading / current_file_content 渲染对应视图
    pub(super) fn render_pf_content(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;

        if self.loading_file_content {
            return placeholder("加载中...", muted_fg);
        }

        let snapshot = match self.current_file_content.as_ref() {
            Some(s) => s,
            None => return placeholder("在左侧选择文件以查看内容", muted_fg),
        };

        if let Some(err) = &snapshot.error {
            return placeholder(err.clone(), gpui::hsla(0.0, 0.65, 0.55, 1.0));
        }
        if snapshot.binary {
            return placeholder("（二进制文件，未渲染内容）", muted_fg);
        }

        let mono = theme.mono_font_family.clone();
        // 文件扩展名决定语法高亮语言（None=纯文本）
        let lang = super::syntax::lang_for_path(&snapshot.path).map(SharedString::from);
        // Rc clone 是引用计数 +1（O(1)），不再每帧拷贝整文件内容
        let lines_rc: Rc<Vec<String>> = snapshot.lines.clone();
        let total = lines_rc.len();

        // gutter 列宽随总行数位数动态
        let digit_count = total.to_string().len().max(2);
        let gutter_w = (digit_count as f32) * 8.0 + 16.0;

        // max_chars 在 select_pf_file 异步路径里算过一次缓存到 snapshot；
        // render 直接读，省去万行文件每帧 100 万次 chars() 迭代
        let content_w = (snapshot.max_chars as f32) * MONO_CHAR_W + 32.0; // 32 = padding
        let total_w = gutter_w + content_w;

        let body = uniform_list(
            "vcs-pf-content",
            total,
            cx.processor({
                let lines_rc = lines_rc.clone();
                let mono = mono.clone();
                move |_this, range: Range<usize>, _w, cx| {
                    let muted_fg = cx.theme().muted_foreground;
                    let fg = cx.theme().foreground;
                    let lang_ref = lang.as_deref();
                    range
                        .map(|i| {
                            render_content_row(
                                i,
                                &lines_rc[i],
                                gutter_w,
                                content_w,
                                lang_ref,
                                mono.clone(),
                                fg,
                                muted_fg,
                                cx,
                            )
                        })
                        .collect::<Vec<_>>()
                }
            }),
        )
        .track_scroll(&self.pf_content_scroll)
        // w(total_w) 设内容总宽（用于水平滚动）；min_w_full 确保至少撑满外层容器，
        // 让短文件场景下右侧空白也归 list 管，wheel 事件能命中并垂直滚动
        .w(px(total_w))
        .min_w_full()
        // 禁止 list 把 wheel dx 当 dy（list 是单 Y 滚，否则 dx 会被劫持垂直滚）
        .restrict_scroll_to_axis()
        .flex_1();

        // 外层 v_flex（含 banner / header），最里层用 overflow_x_scroll 包 list 实现水平滚
        let mut col = v_flex().size_full().min_h_0();
        if snapshot.truncated {
            col = col.child(truncated_banner(muted_fg, fg));
        }
        col.child(header_bar(&snapshot.path, total, muted_fg, fg))
            .child(
                div()
                    .id("vcs-pf-content-h-scroll")
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_x_scroll()
                    // 禁止外层 div 把 wheel dy 当 dx（div 是单 X 滚）
                    .restrict_scroll_to_axis()
                    .track_scroll(&self.pf_content_h_scroll)
                    // 内层 v_flex 同样需要 min_w_full：当 total_w < 容器宽时撑满容器，
                    // 让 list 撑开到容器宽（list 自身也设了 min_w_full 配合）
                    .child(v_flex().min_w_full().w(px(total_w)).h_full().child(body)),
            )
            .into_any_element()
    }
}

/// 顶部信息条：路径 + 行数
fn header_bar(path: &str, total: usize, muted_fg: gpui::Hsla, fg: gpui::Hsla) -> AnyElement {
    h_flex()
        .w_full()
        .flex_none()
        .px(px(12.0))
        .py(px(6.0))
        .gap(px(8.0))
        .items_center()
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(fg)
                .child(path.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(muted_fg)
                .child(format!("{total} 行")),
        )
        .into_any_element()
}

/// 截断提示 banner
fn truncated_banner(muted_fg: gpui::Hsla, _fg: gpui::Hsla) -> AnyElement {
    let mut bg = gpui::hsla(40.0 / 360.0, 0.7, 0.55, 1.0);
    bg.a = 0.10;
    div()
        .w_full()
        .px(px(12.0))
        .py(px(6.0))
        .bg(bg)
        .text_xs()
        .text_color(muted_fg)
        .child("文件较大，仅显示前 4 MB 内容")
        .into_any_element()
}

/// 行号 + 内容（按 lang 语法高亮）。`content_w` 固定为最长行估算，外层 ScrollHandle 同步横滚
#[allow(clippy::too_many_arguments)]
fn render_content_row(
    idx: usize,
    text: &str,
    gutter_w: f32,
    content_w: f32,
    lang: Option<&str>,
    mono: SharedString,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let line_no = idx + 1;
    h_flex()
        .h(px(LINE_HEIGHT))
        .flex_none()
        .child(
            div()
                .flex_none()
                .w(px(gutter_w))
                .px(px(8.0))
                .text_xs()
                .font_family(mono.clone())
                .text_color(muted_fg)
                .child(line_no.to_string()),
        )
        .child(
            div()
                .flex_none()
                .w(px(content_w))
                .px(px(8.0))
                .child(super::syntax::render_code_line(text, lang, fg, mono, cx)),
        )
        .into_any_element()
}

/// 简单文本占位（loading / 空 / 错误 / 二进制）—— 居中显示
fn placeholder(text: impl Into<SharedString>, color: gpui::Hsla) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .child(div().text_sm().text_color(color).child(text.into()))
        .into_any_element()
}
