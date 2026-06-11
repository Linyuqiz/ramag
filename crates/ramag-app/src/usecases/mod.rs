//! Use Cases：编排 domain trait 完成业务用例

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
