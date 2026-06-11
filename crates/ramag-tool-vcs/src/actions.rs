//! VCS 快捷键 Action。绑定在 ramag-bin/main.rs 的 `cx.bind_keys`（context = "VcsView"）

use gpui::Action;
use schemars::JsonSchema;
use serde::Deserialize;

/// 切到 Changes 视图并聚焦 commit message 输入框（默认 cmd-k，仿 IDEA Commit）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_vcs)]
pub struct FocusCommitMessage;

/// 提交暂存区（默认 cmd-enter；仅 commit 输入框聚焦时生效，避免误触）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_vcs)]
pub struct CommitNow;

/// Push 当前分支（默认 cmd-shift-k，仿 IDEA）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_vcs)]
pub struct PushNow;

/// Pull 当前分支（默认 cmd-t，仿 IDEA Update Project）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_vcs)]
pub struct PullNow;

/// 手动刷新工作区状态（默认 cmd-r）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_vcs)]
pub struct RefreshWorkspace;

/// 显示 / 隐藏底部历史面板（默认 cmd-shift-h）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_vcs)]
pub struct ToggleHistoryPane;
