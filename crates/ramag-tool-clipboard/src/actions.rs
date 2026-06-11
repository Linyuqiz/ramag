//! 剪贴板工具快捷键 Action。绑定在 ramag-bin/main.rs 的 `cx.bind_keys`

use gpui::Action;
use schemars::JsonSchema;
use serde::Deserialize;

/// 聚焦搜索框（默认 cmd-f）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_clipboard)]
pub struct FocusClipSearch;

/// 复制选中条目回剪贴板（默认 enter）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_clipboard)]
pub struct CopySelectedClip;

/// 删除选中条目（默认 delete / backspace）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_clipboard)]
pub struct DeleteSelectedClip;

/// 选中项下移（默认 down）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_clipboard)]
pub struct SelectNextClip;

/// 选中项上移（默认 up）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_clipboard)]
pub struct SelectPrevClip;
