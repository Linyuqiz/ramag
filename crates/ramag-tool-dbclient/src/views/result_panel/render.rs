//! `impl Render for ResultPanel` + 警告 banner + 复制操作
//!
//! - SHOW WARNINGS 提示条（仅 MySQL，从 `qr.warnings` 读）
//! - ⌘C 复制选中单元格 / 列名（由 `actions::CopyCellValue` / `CopySelectedColumn` 触发）

use gpui::{
    ClickEvent, ClipboardItem, Context, Focusable as _, IntoElement, ParentElement, Render,
    SharedString, Styled, Window, div, prelude::*,
};
use gpui_component::{
    ActiveTheme, IconName, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    notification::Notification,
    v_flex,
};

use super::ResultPanel;
use super::ResultState;
use super::export::ExportFormat;
use crate::actions::{
    CopyCellValue, CopySelectedColumn, ExportCsv, ExportJson, ExportMarkdown, FindInResults,
};
use crate::views::result_table::render_table;

impl Render for ResultPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 把异步任务挂的通知 push 到全局 toast
        if let Some(n) = self.pending_notification.take() {
            window.push_notification(n, cx);
        }

        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let border = theme.border;
        let secondary_bg = theme.secondary;
        let danger = theme.danger;
        let muted_bg = theme.muted;
        let accent = theme.accent;

        let content = match &self.state {
            ResultState::Empty => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_1()
                .text_color(muted_fg)
                .text_xs()
                .child("点左侧表名查看数据")
                .child("或按 ⌘E 唤出 SQL 编辑器，再按 ⌘↵ 运行")
                .into_any_element(),

            ResultState::Running => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(muted_fg)
                .text_xs()
                .child("执行中...")
                .into_any_element(),

            ResultState::Error(msg) => {
                let msg_for_copy = msg.clone();
                v_flex()
                    .size_full()
                    .p_4()
                    .gap_2()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(danger)
                                    .child("执行失败"),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("copy-error")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Copy)
                                    .tooltip("复制错误信息")
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            msg_for_copy.clone(),
                                        ));
                                        this.pending_notification = Some(
                                            Notification::success("已复制错误信息").autohide(true),
                                        );
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(div().text_xs().text_color(fg).child(msg.clone()))
                    .into_any_element()
            }

            ResultState::Ok(result) => render_table(
                self,
                result.clone(),
                fg,
                muted_fg,
                secondary_bg,
                border,
                muted_bg,
                accent,
                cx,
            )
            .into_any_element(),
        };

        let warnings_banner = self.render_warnings_banner(cx);

        let mut root = v_flex()
            .size_full()
            .min_w_0()
            .on_action(cx.listener(|this, _: &ExportCsv, _, cx| {
                this.export(ExportFormat::Csv, cx);
            }))
            .on_action(cx.listener(|this, _: &ExportJson, _, cx| {
                this.export(ExportFormat::Json, cx);
            }))
            .on_action(cx.listener(|this, _: &FindInResults, window, cx| {
                let handle = this.row_filter_input.read(cx).focus_handle(cx);
                handle.focus(window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CopyCellValue, _, cx| {
                this.copy_selected_cell(cx);
            }))
            .on_action(cx.listener(|this, _: &CopySelectedColumn, _, cx| {
                this.copy_selected_column_name(cx);
            }))
            .on_action(cx.listener(|this, _: &ExportMarkdown, _, cx| {
                this.export(ExportFormat::Markdown, cx);
            }));
        if let Some(banner) = warnings_banner {
            root = root.child(banner);
        }
        root.child(div().flex_1().min_h_0().child(content))
    }
}

impl ResultPanel {
    /// 渲染 SHOW WARNINGS 提示条
    pub(super) fn render_warnings_banner(&self, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        let ResultState::Ok(qr) = &self.state else {
            return None;
        };
        if qr.warnings.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let warning_color = theme.warning;
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let border = theme.border;
        let secondary_bg = theme.secondary;

        let count = qr.warnings.len();
        let expanded = self.warnings_expanded;
        let header_label = if expanded {
            format!("⚠ {count} 条服务端警告（点击收起）")
        } else {
            format!("⚠ {count} 条服务端警告（点击展开）")
        };
        let header = h_flex()
            .id(SharedString::from("warnings-header"))
            .w_full()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .cursor_pointer()
            .bg(secondary_bg)
            .border_b_1()
            .border_color(border)
            .child(
                div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(warning_color)
                    .child(header_label),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.warnings_expanded = !this.warnings_expanded;
                cx.notify();
            }));

        if !expanded {
            return Some(header.into_any_element());
        }

        const MAX_VISIBLE: usize = 20;
        let mut rows: Vec<gpui::AnyElement> =
            Vec::with_capacity(qr.warnings.len().min(MAX_VISIBLE) + 1);
        for w in qr.warnings.iter().take(MAX_VISIBLE) {
            let line = format!("[{} {}] {}", w.level, w.code, w.message);
            rows.push(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(fg)
                    .child(line)
                    .into_any_element(),
            );
        }
        if count > MAX_VISIBLE {
            rows.push(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(muted_fg)
                    .child(format!("…更多 {} 条", count - MAX_VISIBLE))
                    .into_any_element(),
            );
        }

        Some(
            v_flex()
                .w_full()
                .flex_none()
                .border_b_1()
                .border_color(border)
                .child(header)
                .child(v_flex().py_1().children(rows))
                .into_any_element(),
        )
    }

    /// 复制选中单元格完整值
    pub(super) fn copy_selected_cell(&mut self, cx: &mut Context<Self>) {
        let Some((ri, ci)) = self.selected_cell else {
            return;
        };
        let ResultState::Ok(result) = &self.state else {
            return;
        };
        let Some(val) = result.rows.get(ri).and_then(|r| r.values.get(ci)) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(val.to_clipboard_string()));
        self.pending_notification = Some(Notification::success("已复制单元格").autohide(true));
        cx.notify();
    }

    /// 复制选中列的列名
    pub(super) fn copy_selected_column_name(&mut self, cx: &mut Context<Self>) {
        let Some((_, ci)) = self.selected_cell else {
            return;
        };
        let ResultState::Ok(result) = &self.state else {
            return;
        };
        let Some(name) = result.columns.get(ci).cloned() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(name.clone()));
        self.pending_notification =
            Some(Notification::success(format!("已复制列名 {name}")).autohide(true));
        cx.notify();
    }
}
