//! Tokio runtime 桥接
//!
//! GPUI 内部用 smol；sqlx 强依赖 tokio。直接在 GPUI 任务里 await sqlx 会
//! panic（找不到 tokio reactor）。这里维护一个全局 tokio 单例 runtime，
//! 通过 `run_in_tokio` 把 future 派发过去，结果用 oneshot channel 跨 runtime 送回。
//!
//! # 关于 expect
//!
//! - `Builder::build()` 失败 = 系统资源不足（线程/fd 耗尽），不可恢复，让进程崩溃合理
//! - `oneshot::Receiver::await` 失败 = tokio 任务在送回结果前被 drop（tokio runtime 被回收），
//!   理论上不会发生（runtime 是 'static OnceCell），如果发生说明运行时有重大异常
#![allow(clippy::expect_used)]

use std::future::Future;

use futures::channel::oneshot;
use once_cell::sync::OnceCell;
use tokio::runtime::{Builder, Runtime};

static TOKIO_RUNTIME: OnceCell<Runtime> = OnceCell::new();

/// 取（或惰性初始化）全局 tokio runtime
pub fn tokio_runtime() -> &'static Runtime {
    TOKIO_RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ramag-tokio")
            .enable_all()
            .build()
            .expect("failed to build tokio runtime")
    })
}

/// 在 tokio runtime 上跑一个 Future，用 oneshot 把结果送回当前 runtime
pub async fn run_in_tokio<F, T>(fut: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    tokio_runtime().spawn(async move {
        let _ = tx.send(fut.await);
    });
    rx.await.expect("tokio task dropped before sending result")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn singleton() {
        let r1 = tokio_runtime();
        let r2 = tokio_runtime();
        assert!(std::ptr::eq(r1, r2));
    }
}
