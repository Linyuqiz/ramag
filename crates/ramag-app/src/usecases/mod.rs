//! Use Cases：编排 domain trait 完成业务用例

/// 幂等读操作的「连接错误 → 失效缓存 → 重试一次」包装，让闲置断连后的首次读自动恢复。
///
/// 必须用宏而非 async 闭包助手：闭包捕获 &self / &config 后，在 `background_spawn` 等 Send
/// 上下文会触发「Send is not general enough」（async 闭包 + HRTB 的编译器限制）；宏直接把
/// 操作内联进 async fn，无此问题。
///
/// **仅限幂等读**——写操作（set / insert / update / delete / 任意命令）重试可能重复执行。
///
/// 用法：`retry_idempotent_read!(config.id, self.evict_pool(config), self.driver.xxx(..).await)`
///
/// 宏定义在下方各 service 模块声明之前，故按 macro_rules 文本作用域，usecases 下子模块可直接调用
macro_rules! retry_idempotent_read {
    ($conn_id:expr, $evict:expr, $op:expr) => {{
        match $op {
            ::std::result::Result::Err(::ramag_domain::error::DomainError::ConnectionFailed(
                msg,
            )) => {
                ::tracing::warn!(connection_id = %$conn_id, error = %msg, "idempotent read hit connection error, evict + retry once");
                $evict;
                $op
            }
            other => other,
        }
    }};
}

pub mod clip_thumb;
pub mod clipboard_service;
pub mod connection_service;
pub mod export;
pub mod mongo_service;
pub mod redis_service;

pub use clipboard_service::{CaptureDecision, ClipboardService, decide_capture};
pub use connection_service::ConnectionService;
pub use mongo_service::MongoService;
pub use redis_service::RedisService;
