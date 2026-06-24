//! Redis 视图专属快捷键 Action。绑定在 ramag-bin/main.rs 的 `cx.bind_keys`

use gpui::Action;
use schemars::JsonSchema;
use serde::Deserialize;

/// 切换底部命令行控制台显隐（默认 cmd-e；scoped 到 RedisSession 上下文）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag_redis)]
pub struct ToggleRedisConsole;
