//! 结果集导出：CSV（基于扁平表格）/ JSON（原始文档）。
//! rfd 保存框阻塞，放 std::thread 跑，结果经 oneshot 回主线程（与 dbclient 同款）。

use std::path::PathBuf;

use futures::channel::oneshot;
use gpui::Context;
use gpui_component::notification::Notification;

use super::ResultPanel;
use super::flatten::FlatTable;

impl ResultPanel {
    /// 导出当前结果：as_csv=true 导 CSV（基于扁平表格），否则导 JSON（原始文档）
    pub(crate) fn export_documents(&mut self, as_csv: bool, cx: &mut Context<Self>) {
        let Some(result) = self.result.as_ref() else {
            return self.notify_error("无可导出的结果".to_string(), cx);
        };
        if result.documents.is_empty() {
            return self.notify_error("结果为空，无需导出".to_string(), cx);
        }
        let (content, ext) = if as_csv {
            match &self.table {
                Some(t) => (flat_to_csv(t), "csv"),
                None => return self.notify_error("无表格数据可导出 CSV".to_string(), cx),
            }
        } else {
            (
                serde_json::to_string_pretty(&result.documents).unwrap_or_default(),
                "json",
            )
        };
        let coll = self
            .target_collection
            .clone()
            .unwrap_or_else(|| "export".to_string());
        let name = format!("{coll}.{ext}");
        // rfd 保存框是阻塞的：放 std::thread 跑，结果经 oneshot 回主线程（与 dbclient 同款）
        let (tx, rx) = oneshot::channel::<ExportOutcome>();
        std::thread::spawn(move || {
            let path = rfd::FileDialog::new()
                .set_file_name(&name)
                .add_filter(ext, &[ext])
                .save_file();
            let outcome = match path {
                None => ExportOutcome::Cancelled,
                Some(p) => match std::fs::write(&p, content) {
                    Ok(_) => ExportOutcome::Saved(p),
                    Err(e) => ExportOutcome::Failed(e.to_string()),
                },
            };
            let _ = tx.send(outcome);
        });
        cx.spawn(async move |this, cx| {
            let outcome = rx.await.unwrap_or(ExportOutcome::Cancelled);
            let _ = this.update(cx, |this, cx| {
                this.pending_notification = Some(match outcome {
                    ExportOutcome::Saved(p) => Notification::success(
                        p.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "导出完成".to_string()),
                    )
                    .title("导出成功")
                    .autohide(true),
                    ExportOutcome::Cancelled => Notification::info("已取消导出").autohide(true),
                    ExportOutcome::Failed(e) => {
                        Notification::error(e).title("导出失败").autohide(true)
                    }
                });
                cx.notify();
            });
        })
        .detach();
    }
}

/// rfd 文件保存结果（线程 → 主线程）
enum ExportOutcome {
    Saved(PathBuf),
    Cancelled,
    Failed(String),
}

/// FlatTable → CSV（列头 path + 行，逗号/引号/换行转义）
fn flat_to_csv(table: &FlatTable) -> String {
    let mut out = String::new();
    let header: Vec<String> = table.columns.iter().map(|c| csv_escape(&c.path)).collect();
    out.push_str(&header.join(","));
    out.push('\n');
    for row in &table.rows {
        let cells: Vec<String> = row.iter().map(|c| csv_escape(&c.text)).collect();
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    out
}

/// CSV 字段转义：含逗号 / 引号 / 换行时用双引号包裹，内部引号翻倍
fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
