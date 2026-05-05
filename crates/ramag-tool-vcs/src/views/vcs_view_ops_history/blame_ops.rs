//! VcsView blame 相关 ops：行级 inline blame banner / 完整 blame 加载 / 切换 diff↔blame 视图

use gpui::{Context, SharedString};
use tracing::error;

use super::super::vcs_view::VcsView;

impl VcsView {
    /// 行号点击 → 拉当前文件 blame，命中行写到顶部 banner（_is_old 占位，预留 HEAD 侧逻辑）
    pub(crate) fn show_inline_blame(
        &mut self,
        line_no: u32,
        _is_old: bool,
        cx: &mut Context<Self>,
    ) {
        let path = self
            .selected_file
            .as_ref()
            .map(|(p, _)| p.clone())
            .or_else(|| self.selected_commit_file.clone())
            .or_else(|| self.selected_pf_path.clone());
        let Some(path) = path else {
            return;
        };
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        if self.inline_blame_text.as_deref() == Some("加载行作者信息...") {
            return;
        }
        self.inline_blame_text = Some("加载行作者信息...".into());
        cx.notify();
        let driver = self.driver.clone();
        cx.spawn(async move |this, cx| {
            let result = driver.blame(&repo, &path).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(lines) => {
                        if let Some(b) = lines.iter().find(|l| l.line_no == line_no) {
                            let short = b.commit.0.chars().take(7).collect::<String>();
                            let date = b.timestamp.format("%Y-%m-%d");
                            this.inline_blame_text = Some(SharedString::from(format!(
                                "L{line_no}　{short}　·　{}　·　{date}　·　{}",
                                b.author, b.subject
                            )));
                        } else {
                            this.inline_blame_text =
                                Some(SharedString::from(format!("L{line_no}：未找到 blame 信息")));
                        }
                    }
                    Err(e) => {
                        error!(error = %e, %path, "vcs: inline blame failed");
                        this.inline_blame_text =
                            Some(SharedString::from(format!("blame 失败：{e}")));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 清空 inline blame banner（用户切文件 / 关闭按钮 / 切视图时调）
    pub(crate) fn clear_inline_blame(&mut self, cx: &mut Context<Self>) {
        if self.inline_blame_text.is_some() {
            self.inline_blame_text = None;
            cx.notify();
        }
    }

    /// 切换 diff/blame 视图；showing_blame=true 拉 blame，否则清空
    /// 路径优先取 selected_file（Changes），其次 selected_commit_file（commit tab）
    pub(crate) fn toggle_blame(&mut self, cx: &mut Context<Self>) {
        self.showing_blame = !self.showing_blame;
        if self.showing_blame {
            let path = self
                .selected_file
                .as_ref()
                .map(|(p, _)| p.clone())
                .or_else(|| self.selected_commit_file.clone());
            if let Some(p) = path {
                self.load_blame(p, cx);
            } else {
                self.showing_blame = false;
            }
        } else {
            self.blame_lines.clear();
        }
        cx.notify();
    }

    /// 异步拉取指定文件的 blame
    pub(crate) fn load_blame(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.loading_blame = true;
        self.blame_lines = Vec::new();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = driver.blame(&repo, &path).await;
            let _ = this.update(cx, |this, cx| {
                this.loading_blame = false;
                match result {
                    Ok(lines) => this.blame_lines = lines,
                    Err(e) => {
                        error!(error = %e, %path, "vcs: blame failed");
                        this.error = Some(format!("Blame 失败：{e}"));
                        this.showing_blame = false;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
