//! Tokio runtime 桥接
//!
//! # 背景
//!
//! GPUI 内部使用 smol runtime；sqlx 强依赖 tokio runtime。
//! 直接在 GPUI 的异步任务里调用 sqlx 会 panic（因为找不到 tokio reactor）。
//!
//! # 解决方案
//!
//! 1. 启动一个独立的 tokio multi-thread runtime（程序生命周期内单例）
//! 2. 调用 sqlx 时，把 future 通过 `block_in_runtime` 派发到 tokio runtime
//! 3. 用 `futures::channel::oneshot` 把结果送回调用方（runtime 无关，安全跨）
//!
//! # 用法
//!
//! ```ignore
//! use ramag_infra_mysql::runtime::run_in_tokio;
//!
//! let result = run_in_tokio(async {
//!     sqlx::query("SELECT 1").fetch_one(&pool).await
//! }).await;
//! ```

use std::future::Future;

use futures::channel::oneshot;
use once_cell::sync::OnceCell;
use tokio::runtime::{Builder, Runtime};

/// 全局 tokio runtime 单例
static TOKIO_RUNTIME: OnceCell<Runtime> = OnceCell::new();

/// 获取（或惰性初始化）tokio runtime
pub fn tokio_runtime() -> &'static Runtime {
    TOKIO_RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ramag-tokio")
            .enable_all()
            .build()
            .expect("无法创建 tokio runtime")
    })
}

/// 在 tokio runtime 中执行一个 Future，并将结果通过 oneshot channel 返回
///
/// 调用方可以是任何 runtime（smol / tokio / async-std），只要 await 这个函数返回的 Future。
///
/// # Panic
///
/// 如果 oneshot 在结果送达前被 drop（理论上不应发生），将返回错误。
pub async fn run_in_tokio<F, T>(fut: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = oneshot::channel();

    tokio_runtime().spawn(async move {
        let result = fut.await;
        // 接收方可能已经 drop（如 GPUI 任务被取消），这里忽略发送错误
        let _ = tx.send(result);
    });

    rx.await.expect("tokio runtime 内任务未送回结果（异常情况）")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokio_runtime_singleton() {
        // 多次调用返回同一个 runtime（内存地址相同）
        let r1 = tokio_runtime();
        let r2 = tokio_runtime();
        assert!(std::ptr::eq(r1, r2));
    }

    #[tokio::test]
    async fn run_in_tokio_basic() {
        let v = run_in_tokio(async { 42 }).await;
        assert_eq!(v, 42);
    }
}
