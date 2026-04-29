//! 跨工具共用的全局 Action
//!
//! 这里只放真正"工具无关"的快捷动作（比如关闭当前 tab 这种各 Tool 都需要的）；
//! 工具自己专属的 Action（如 RunQuery / RefreshKeyTree）仍放在各自的 actions 模块

use gpui::Action;
use schemars::JsonSchema;
use serde::Deserialize;

/// 关闭当前 Tab（默认 ⌘W / Cmd+W）
///
/// 由各 Tool 的 Session/Panel 监听：
/// - 有可关闭 tab → 消费事件并关闭
/// - 没有可关闭 tab → cx.propagate() 让事件冒泡到 main.rs 的全局 fallback 关窗
///
/// macOS 习惯：关最后一个窗后保留 app（dock 图标点击重开）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag)]
pub struct CloseTab;
