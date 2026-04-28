//! Tokio runtime 桥接（与 ramag-infra-mysql 同形态）
//!
//! redis-rs 的异步连接强依赖 tokio runtime；GPUI 内部用 smol。
//! 直接在 GPUI 的异步任务里调用 redis-rs 会 panic（找不到 tokio reactor）。
//!
//! 本模块创建一个独立的 tokio multi-thread runtime（程序生命周期内单例），
//! 把所有 redis 操作派发到该 runtime，结果通过 `futures::channel::oneshot` 送回。
//!
//! 与 mysql 各持有一份运行时，互不干扰；线程数各 2 个，总开销可控。

use std::future::Future;

use futures::channel::oneshot;
use once_cell::sync::OnceCell;
use tokio::runtime::{Builder, Runtime};

/// 全局 tokio runtime 单例（redis 专用）
static TOKIO_RUNTIME: OnceCell<Runtime> = OnceCell::new();

/// 获取（或惰性初始化）tokio runtime
///
/// 创建失败属于初始化级故障（OS 拒绝创建线程池），程序无法继续，
/// 此处用 expect 显式 panic；与 mysql 同款 runtime 处理一致
#[allow(clippy::expect_used)]
pub fn tokio_runtime() -> &'static Runtime {
    TOKIO_RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ramag-redis-tokio")
            .enable_all()
            .build()
            .expect("无法创建 redis 专用 tokio runtime")
    })
}

/// 在 tokio runtime 中执行一个 Future，并将结果通过 oneshot channel 返回
///
/// 调用方可以是任何 runtime（smol / tokio / async-std），只要 await 这个函数返回的 Future。
///
/// `rx.await` 的 Err 仅在 sender 被 drop 时出现（比如 spawn 内部 panic），
/// 这是不可恢复的 bug 状态，用 expect 暴露问题
#[allow(clippy::expect_used)]
pub async fn run_in_tokio<F, T>(fut: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = oneshot::channel();

    tokio_runtime().spawn(async move {
        let result = fut.await;
        let _ = tx.send(result);
    });

    rx.await
        .expect("redis tokio runtime 内任务未送回结果（异常情况）")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokio_runtime_singleton() {
        let r1 = tokio_runtime();
        let r2 = tokio_runtime();
        assert!(std::ptr::eq(r1, r2));
    }

    #[tokio::test]
    async fn run_in_tokio_basic() {
        let v = run_in_tokio(async { 42i32 }).await;
        assert_eq!(v, 42);
    }
}
