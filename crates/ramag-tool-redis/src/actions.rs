//! Redis Tool 的 GPUI Actions（命名空间 `ramag_redis`）
//!
//! 暂仅声明几个高频快捷动作；后续 Stage 17/18 加 CLI 面板时再扩

use gpui::Action;
use schemars::JsonSchema;
use serde::Deserialize;

/// 刷新当前 Key 树（重发 SCAN 0 起的迭代）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_redis)]
pub struct RefreshKeyTree;

/// 删除当前选中的 Key（带二次确认）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_redis)]
pub struct DeleteCurrentKey;
