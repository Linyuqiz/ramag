//! Pub/Sub 实时消息实体
//!
//! Driver 在后台 task 中读取 redis-rs 的 PubSub 流，把每条消息封装成
//! [`PubSubMessage`] 并推到 `futures::channel::mpsc::UnboundedSender`；
//! 调用方持有 `UnboundedReceiver`，drop 即取消订阅（driver 任务发现
//! 通道关闭后退出）

use serde::{Deserialize, Serialize};

/// 单条 Pub/Sub 消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PubSubMessage {
    /// 实际消息所在的 channel 名（即使是 PSUBSCRIBE 也是命中的具体 channel）
    pub channel: String,
    /// PSUBSCRIBE 模式订阅时的匹配模式；普通 SUBSCRIBE 为 None
    pub pattern: Option<String>,
    /// 消息体（utf-8；二进制内容由 driver 端尽力转字符串）
    pub payload: String,
    /// 客户端接收时间（自 epoch 毫秒）
    pub received_at_ms: i64,
}
