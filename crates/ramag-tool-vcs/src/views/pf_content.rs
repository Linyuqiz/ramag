//! Project Files 视图主区：渲染选中文件的**内容**（与 diff 视图独立）
//!
//! 数据来源：[`super::vcs_view::VcsView::current_file_content`]，由
//! [`super::vcs_view_ops_repo::select_pf_file`] 异步读盘后写入。
//!
//! 渲染要点：
//! - 行号 + 内容两列，等宽 mono 字体
//! - **垂直**虚拟化：`uniform_list` 行级（22px 等高），万行文件也流畅
//! - **水平**滚动：外层 `div.overflow_x_scroll().track_scroll(h_scroll)` 包定宽 `v_flex`，
//!   与 dbclient `result_table` 同款方案——uniform_list 只管 Y，X 轴由外层 div 处理
//! - 二进制文件 / 读取失败 / 大文件截断 都给出独立占位 banner
//! - 无内容选中时显示「请在左侧选择文件」提示

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement, SharedString, Styled,
    div, prelude::*, px, uniform_list,
};
use gpui_component::{ActiveTheme, h_flex, v_flex};

use super::vcs_view::VcsView;

/// 关闭 GPUI 单轴 scroll 元素的"另一方向劫持"行为（同 dbclient::result_table 私有 trait）
///
/// GPUI 默认：overflow.x=Scroll 且 overflow.y!=Scroll 时，wheel 的 dy 会被当成 dx 应用，
/// 结果是"往下滚 → 横向滚到底"。设置 `restrict_scroll_to_axis = true` 让 wheel 严格按方向消费。
trait RestrictScrollExt: Styled + Sized {
    fn restrict_scroll_to_axis(mut self) -> Self {
        self.style().restrict_scroll_to_axis = Some(true);
        self
    }
}
impl<T: Styled> RestrictScrollExt for T {}

/// 单行高度（与 diff_panel 视觉一致 22px，便于切换视图无视差）
const LINE_HEIGHT: f32 = 22.0;
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
                    range
                        .map(|i| {
                            render_content_row(
                                i,
                                &lines_rc[i],
                                gutter_w,
                                content_w,
                                mono.clone(),
                                fg,
                                muted_fg,
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

/// 单行渲染：行号 + 内容（uniform_list closure 内调）
///
/// 行内容设固定宽度 `content_w`（最长行估算）+ whitespace_nowrap，
/// 让 uniform_list 行宽稳定，外层水平滚 ScrollHandle 才能在所有行同步。
fn render_content_row(
    idx: usize,
    text: &str,
    gutter_w: f32,
    content_w: f32,
    mono: SharedString,
    fg: gpui::Hsla,
    muted_fg: gpui::Hsla,
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
                .text_xs()
                .font_family(mono)
                .text_color(fg)
                .whitespace_nowrap()
                .child(text.to_string()),
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
