//! 跨工具共用 Action

use gpui::Action;
use schemars::JsonSchema;
use serde::Deserialize;

/// `cmd-w` 关 Tab。各 Tool Session 先消费，没消费则冒泡到 main.rs 关窗
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag)]
pub struct CloseTab;
