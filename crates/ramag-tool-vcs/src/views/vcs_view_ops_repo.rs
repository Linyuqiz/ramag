//! 仓库 / Session 管理 ops：pick_directory / open_recent_repo / remove_recent_repo /
//! remove_open_repo / open_repo_async（共享异步流：open + 拉 status / 分支 / stash 等）

use gpui::prelude::*;
use gpui::{ClickEvent, Context, Window};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{ActiveTheme, Sizable as _, WindowExt as _};
use ramag_domain::entities::{BranchKind, RepoConfig, RepoId};
use tracing::{error, info};

use super::helpers::{ActiveView, FileContentSnapshot, FileTab, FileTabSource, FilesViewMode};
use super::vcs_view::{RepoSessionState, VcsView};

/// Project Files 点击文件后读盘上限（4MB）；超过截断后 UI 显示提示
const PF_FILE_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// worker 线程跨线程返回结构（Send）；主线程 finalize 后包 Rc 成 FileContentSnapshot
struct RawFileContent {
    path: String,
    lines: Vec<String>,
    truncated: bool,
    binary: bool,
    error: Option<String>,
}

impl VcsView {
    /// 弹出系统目录选择器；用户选完后异步打开仓库
    pub(super) fn pick_directory(&mut self, cx: &mut Context<Self>) {
        let driver = self.driver.clone();
        self.loading = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let dialog = rfd::FileDialog::new().set_title("选择 Git 仓库目录");
            let Some(path) = dialog.pick_folder() else {
                let _ = this.update(cx, |this, cx| {
                    this.loading = false;
                    cx.notify();
                });
                return;
            };
            open_repo_async(&this, driver, path, cx).await;
        })
        .detach();
    }

    /// 从最近列表点击仓库行 → 直接打开（不弹文件对话框）
    pub(super) fn open_recent_repo(&mut self, path: String, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        let driver = self.driver.clone();
        let pb = std::path::PathBuf::from(path);
        self.loading = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            open_repo_async(&this, driver, pb, cx).await;
        })
        .detach();
    }

    /// 从最近列表移除（不删磁盘）；按 path 找 RepoId 后调 storage.delete_repo
    pub(super) fn remove_recent_repo(&mut self, path: String, cx: &mut Context<Self>) {
        let repo_id = self
            .recent_repos
            .iter()
            .find(|r| r.path == path)
            .map(|r| r.id.clone());
        self.recent_repos.retain(|r| r.path != path);
        if let Some(id) = repo_id {
            self.delete_repo_async(id, cx);
        }
        cx.notify();
    }

    /// 刷新 Files panel 当前视图（Changes/Stash/Project 各调对应 reload）
    pub(super) fn refresh_current_files_view(&mut self, cx: &mut Context<Self>) {
        match self.files_view_mode {
            FilesViewMode::Changes => self.reload_status_silent(cx),
            FilesViewMode::Stash => self.reload_stashes(cx),
            FilesViewMode::Project => self.reload_project_files(cx),
        }
    }

    /// 异步拉 Project Files（git ls-files：tracked + 未 ignore 的 untracked）
    pub(super) fn reload_project_files(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        self.loading_project_files = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = driver.list_files(&repo).await;
            let _ = this.update(cx, |this, cx| {
                this.loading_project_files = false;
                match result {
                    Ok(mut paths) => {
                        // 字母序：让目录树渲染稳定（同一目录文件聚拢）
                        paths.sort();
                        this.project_files = paths;
                    }
                    Err(e) => {
                        error!(error = %e, "vcs: list project files failed");
                        // 失败时仍清空避免显示旧数据；错误以 banner 形式提示
                        this.project_files = Vec::new();
                        this.error = Some(format!("加载 Project Files 失败: {e}"));
                    }
                }
                // 列表内容变了 → 递增版本号让 render 缓存失效
                this.project_files_version = this.project_files_version.wrapping_add(1);
                cx.notify();
            });
        })
        .detach();
    }

    /// Project Files 模式点击文件 → 走统一 file_tabs：命中已开 tab 直接激活；新文件追加 tab 后异步读盘
    ///
    /// - 异步读盘走 std::thread + oneshot（与 ramag-infra-git/runtime 同款，不阻塞 GPUI 线程）
    /// - 4MB 上限防大文件 / NUL 字节检测识别二进制
    pub(super) fn select_pf_file(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref() else {
            return;
        };
        // 点击 Project Files 文件 → 关掉 commit detail，避免主区残留 commit diff
        if self.viewing_commit.is_some() {
            self.viewing_commit = None;
            self.commit_files.clear();
            self.commit_files_collapsed.clear();
            self.selected_commit_file = None;
            self.commit_file_diff = None;
            self.loading_commit_files = false;
        }
        let repo_path = repo.path.clone();
        let idx = if let Some(i) = self
            .file_tabs
            .iter()
            .position(|t| t.path == path && t.source == FileTabSource::ProjectFiles)
        {
            i
        } else {
            self.file_tabs.push(FileTab {
                path: path.clone(),
                source: FileTabSource::ProjectFiles,
                cached_diff: None,
                cached_content: None,
            });
            self.file_tabs.len() - 1
        };
        self.active_file_tab_idx = Some(idx);
        let tab = self.file_tabs[idx].clone();
        self.activate_file_tab_state(tab.clone());
        cx.notify();
        if tab.cached_content.is_some() {
            return;
        }

        let abs_path = std::path::PathBuf::from(&repo_path).join(&path);
        cx.spawn(async move |this, cx| {
            let (tx, rx) = futures::channel::oneshot::channel();
            let path_for_thread = path.clone();
            std::thread::spawn(move || {
                let raw = read_raw_file_content(&abs_path, &path_for_thread);
                let _ = tx.send(raw);
            });
            let raw = rx.await.ok();
            let _ = this.update(cx, |this, cx| {
                let snapshot = raw.map(finalize_file_snapshot);
                if let Some(tab) = this
                    .file_tabs
                    .iter_mut()
                    .find(|t| t.path == path && t.source == FileTabSource::ProjectFiles)
                {
                    tab.cached_content = snapshot.clone();
                }
                if this.selected_pf_path.as_deref() == Some(path.as_str()) {
                    this.loading_file_content = false;
                    this.current_file_content = snapshot;
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 静默拉一次工作区状态（不显 loading 占整屏，仅写回 self.status）
    pub(super) fn reload_status_silent(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.as_ref().map(|r| r.id.clone()) else {
            return;
        };
        let driver = self.driver.clone();
        cx.spawn(async move |this, cx| {
            let new_status = driver.status(&repo).await.ok();
            let _ = this.update(cx, |this, cx| {
                if let Some(s) = new_status {
                    this.status = Some(s);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 关闭指定路径的 tab；若是当前 tab 则尝试切到下一个，否则直接移除
    pub(super) fn remove_open_repo(&mut self, path: String, cx: &mut Context<Self>) {
        let is_current = self.repo.as_ref().map(|r| r.path == path).unwrap_or(false);
        self.open_repos.retain(|r| r.path != path);
        if is_current {
            if let Some(next) = self.open_repos.first().cloned() {
                self.open_recent_repo(next.path, cx);
            } else {
                self.reset_session_state(cx);
            }
        } else {
            cx.notify();
        }
    }

    fn reset_session_state(&mut self, cx: &mut Context<Self>) {
        self.repo = None;
        self.status = None;
        self.local_branches.clear();
        self.remote_branches.clear();
        self.history_commits.clear();
        self.viewing_commit = None;
        self.commit_files.clear();
        self.selected_commit_file = None;
        self.commit_file_diff = None;
        self.selected_file = None;
        self.current_diff = None;
        self.file_tabs.clear();
        self.active_file_tab_idx = None;
        self.active_view = ActiveView::RepoList;
        cx.notify();
    }

    /// 把当前仓库的文件 tab + commit 草稿状态保存到缓存（切换仓库前调用）
    ///
    /// commit_input 的当前文本同时入快照——切回该仓库时再原样恢复，避免跨仓库串扰
    pub(super) fn save_current_session_to_cache(&mut self, cx: &gpui::App) {
        let Some(path) = self.repo.as_ref().map(|r| r.path.clone()) else {
            return;
        };
        if let (Some(idx), Some(diff)) = (self.active_file_tab_idx, self.current_diff.clone())
            && let Some(tab) = self.file_tabs.get_mut(idx)
        {
            tab.cached_diff = Some(diff);
        }
        let commit_text = self.commit_input.read(cx).value();
        self.repo_session_cache.insert(
            path,
            RepoSessionState {
                file_tabs: self.file_tabs.clone(),
                active_file_tab_idx: self.active_file_tab_idx,
                commit_text,
                commit_amend: self.commit_amend,
                commit_sign: self.commit_sign,
            },
        );
    }

    /// 从缓存还原文件 tab + commit 面板状态；commit 文本通过 pending_commit_text 让
    /// Render 阶段（持有 Window）写回 InputState。返回 true 表示命中缓存
    pub(super) fn restore_session_from_cache(&mut self, path: &str) -> bool {
        let cached = self.repo_session_cache.get(path).cloned();
        match cached {
            Some(state) => {
                self.file_tabs = state.file_tabs;
                self.active_file_tab_idx = state.active_file_tab_idx;
                self.commit_amend = state.commit_amend;
                self.commit_sign = state.commit_sign;
                // 即使文本相同也写：保证 Render 一定走 set_value 覆盖前一个仓库残留
                self.pending_commit_text = Some(state.commit_text);
                if let Some(idx) = self.active_file_tab_idx
                    && let Some(tab) = self.file_tabs.get(idx).cloned()
                {
                    self.activate_file_tab_state(tab);
                }
                true
            }
            None => {
                // 全新仓库：清空 commit 面板，避免延续上一个仓库的草稿 / amend / sign
                self.commit_amend = false;
                self.commit_sign = false;
                self.pending_commit_text = Some(gpui::SharedString::default());
                false
            }
        }
    }
}

/// 共享逻辑：实际打开 repo + 拉 status / 分支 / stash / tag / remote
///
/// 由 [`VcsView::pick_directory`] 与 [`VcsView::open_recent_repo`] 共用：
/// 前者从文件对话框得到 path，后者从最近列表得到 path，之后流程完全一样
pub(super) async fn open_repo_async(
    this: &gpui::WeakEntity<VcsView>,
    driver: std::sync::Arc<dyn ramag_domain::traits::GitDriver>,
    path: std::path::PathBuf,
    cx: &mut gpui::AsyncApp,
) {
    info!(?path, "vcs: opening repo");
    let open_result = driver.open_repo(&path).await;
    let repo_config = match open_result {
        Ok(r) => r,
        Err(e) => {
            error!(error = %e, "vcs: open repo failed");
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                this.error = Some(format!("打开仓库失败: {e}"));
                cx.notify();
            });
            return;
        }
    };

    let id = repo_config.id.clone();
    let status_fut = driver.status(&id);
    let local_fut = driver.list_branches(&id, BranchKind::Local);
    let remote_fut = driver.list_branches(&id, BranchKind::Remote);
    let (status, local, remote) = futures::future::join3(status_fut, local_fut, remote_fut).await;

    let _ = this.update(cx, |this, cx| {
        this.loading = false;
        let mut repo_config = repo_config;
        repo_config.last_opened_at = Some(chrono::Utc::now());
        // 是否首次打开（区分「新开仓库」和「tab 切换」）
        let is_new = !this.open_repos.iter().any(|r| r.path == repo_config.path);
        this.save_current_session_to_cache(cx);
        if is_new && !this.recent_repos.iter().any(|r| r.path == repo_config.path) {
            // 全新仓库：追加到列表末尾，再按名字排序保持稳定顺序
            this.recent_repos.push(repo_config.clone());
            this.recent_repos.sort_by(|a, b| a.name.cmp(&b.name));
        }
        this.save_repo_async(repo_config.clone(), cx);
        this.clear_session_data();

        this.repo = Some(repo_config.clone());
        if is_new {
            this.open_repos.push(repo_config.clone());
        }
        this.status = status.ok();
        this.local_branches = local.unwrap_or_default();
        this.remote_branches = remote.unwrap_or_default();
        this.active_view = ActiveView::Session;

        // 已访问过的仓库：还原文件 tab 状态；新仓库：空 tabs 让用户自己选
        this.restore_session_from_cache(&repo_config.path);
        cx.notify();
        this.reload_stashes(cx);
        this.reload_tags(cx);
        this.reload_remotes(cx);
        this.reload_project_files(cx);
        // 切仓库后 clear_session_data 已清空 history_commits；若下半 pane 处于打开态，
        // 立即拉新仓库首页，避免用户看到「空 commit 列表」（原行为只有手动 toggle 才 lazy load）
        if this.history_pane_visible && this.repo.is_some() {
            this.load_history_page(0, cx);
        }
    });
}

impl VcsView {
    /// 保存单条 RepoConfig 到 storage（失败仅 warn，不阻塞 UI）
    pub(super) fn save_repo_async(&self, repo: RepoConfig, cx: &mut Context<Self>) {
        let storage = self.storage.clone();
        cx.background_spawn(async move {
            if let Err(e) = storage.save_repo(&repo).await {
                tracing::warn!(error = %e, repo = %repo.name, "vcs: save_repo failed");
            }
        })
        .detach();
    }

    /// 从 storage 删除单条 RepoConfig（失败仅 warn）
    pub(super) fn delete_repo_async(&self, id: RepoId, cx: &mut Context<Self>) {
        let storage = self.storage.clone();
        cx.background_spawn(async move {
            if let Err(e) = storage.delete_repo(&id).await {
                tracing::warn!(error = %e, repo_id = %id, "vcs: delete_repo failed");
            }
        })
        .detach();
    }

    /// 弹确认对话框：从最近列表移除仓库（不删磁盘文件）
    pub(super) fn confirm_remove_recent_repo(
        &self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity();
        let name = self
            .recent_repos
            .iter()
            .find(|r| r.path == path)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| path.clone());
        let path_ok = path.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let path = path_ok.clone();
            let desc =
                format!("确定从最近列表移除「{name}」吗？\n仅清除本地最近记录，不会删除磁盘文件。");
            let cancel = Button::new("vcs-repo-del-cancel")
                .ghost()
                .small()
                .label("取消")
                .on_click(|_: &ClickEvent, window, app| window.close_dialog(app));
            let ok = Button::new("vcs-repo-del-ok")
                .danger()
                .small()
                .label("移除")
                .on_click({
                    let view = view.clone();
                    let path = path.clone();
                    move |_: &ClickEvent, window, app| {
                        view.update(app, |this, cx| this.remove_recent_repo(path.clone(), cx));
                        window.close_dialog(app);
                    }
                });
            dialog
                .title("从最近列表移除？")
                .margin_top(gpui::px(180.0))
                .content(move |c, _, cx| {
                    c.child(
                        gpui::div()
                            .py(gpui::px(4.0))
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(desc.clone()),
                    )
                })
                .footer(
                    gpui_component::h_flex()
                        .w_full()
                        .justify_end()
                        .gap(gpui::px(8.0))
                        .child(cancel)
                        .child(ok),
                )
        });
    }

    /// 异步 Clone 远程仓库到本地路径，完成后复用 open_repo_async 走 open + 拉数据流
    pub(super) fn clone_repo_async(
        &mut self,
        url: String,
        dest: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let driver = self.driver.clone();
        self.loading = true;
        self.error = None;
        self.show_clone_panel = false;
        cx.notify();
        cx.spawn(
            async move |this, cx| match driver.clone_repo(&url, &dest).await {
                Ok(rc) => {
                    tracing::info!(url = %url, dest = ?dest, "vcs: clone done");
                    open_repo_async(&this, driver, std::path::PathBuf::from(&rc.path), cx).await;
                }
                Err(e) => {
                    tracing::error!(error = %e, "vcs: clone failed");
                    let _ = this.update(cx, |this, cx| {
                        this.loading = false;
                        this.error = Some(format!("Clone 失败: {e}"));
                        cx.notify();
                    });
                }
            },
        )
        .detach();
    }

    /// 异步初始化空仓库，完成后打开 session
    pub(super) fn init_repo_async(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        let driver = self.driver.clone();
        self.loading = true;
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            open_repo_async(&this, driver, path, cx).await;
        })
        .detach();
    }

    /// 启动时从 storage 加载 recent_repos（跨重启保留）
    pub(super) fn load_recent_repos_async(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let storage = match this.update(cx, |this, _| this.storage.clone()) {
                Ok(s) => s,
                Err(_) => return,
            };
            let result = storage.list_repos().await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(list) => {
                    this.recent_repos = list;
                    cx.notify();
                }
                Err(e) => tracing::warn!(error = %e, "vcs: list_repos failed"),
            });
        })
        .detach();
    }
}

/// 在 worker 线程同步读盘 + 二进制 / 截断检测 → 跨线程 Send 的 [`RawFileContent`]
///
/// 读盘失败（路径不存在 / 权限不足）→ raw.error 携带消息，UI 渲染层提示
fn read_raw_file_content(abs: &std::path::Path, rel: &str) -> RawFileContent {
    let metadata = match std::fs::metadata(abs) {
        Ok(m) => m,
        Err(e) => {
            return RawFileContent {
                path: rel.to_string(),
                lines: Vec::new(),
                truncated: false,
                binary: false,
                error: Some(format!("无法访问文件: {e}")),
            };
        }
    };
    if !metadata.is_file() {
        return RawFileContent {
            path: rel.to_string(),
            lines: Vec::new(),
            truncated: false,
            binary: false,
            error: Some("不是普通文件（可能是软链接 / 设备文件）".into()),
        };
    }
    let total_size = metadata.len();
    let truncated = total_size > PF_FILE_MAX_BYTES;
    // 截断时仅读前 PF_FILE_MAX_BYTES 字节，避免一口气读 100MB 大 log
    let read_result = if truncated {
        read_first_bytes(abs, PF_FILE_MAX_BYTES as usize)
    } else {
        std::fs::read(abs)
    };
    let bytes = match read_result {
        Ok(b) => b,
        Err(e) => {
            return RawFileContent {
                path: rel.to_string(),
                lines: Vec::new(),
                truncated: false,
                binary: false,
                error: Some(format!("读取文件失败: {e}")),
            };
        }
    };
    // 二进制识别：前 8KB 任一字节为 NUL → 不渲染内容
    let head_len = bytes.len().min(8192);
    if bytes[..head_len].contains(&0) {
        return RawFileContent {
            path: rel.to_string(),
            lines: Vec::new(),
            truncated: false,
            binary: true,
            error: None,
        };
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
    RawFileContent {
        path: rel.to_string(),
        lines,
        truncated,
        binary: false,
        error: None,
    }
}

/// 主线程 finalize：算 max_chars + 包 Rc → FileContentSnapshot
fn finalize_file_snapshot(raw: RawFileContent) -> FileContentSnapshot {
    let max_chars = raw
        .lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);
    FileContentSnapshot {
        path: raw.path,
        lines: std::rc::Rc::new(raw.lines),
        max_chars,
        truncated: raw.truncated,
        binary: raw.binary,
        error: raw.error,
    }
}

/// 读取文件前 `limit` 字节（用于大文件截断预览）
fn read_first_bytes(path: &std::path::Path, limit: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let file = std::fs::File::open(path)?;
    let mut buf = Vec::with_capacity(limit);
    file.take(limit as u64).read_to_end(&mut buf)?;
    Ok(buf)
}
