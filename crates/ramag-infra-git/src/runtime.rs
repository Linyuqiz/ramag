//! 同步 → async 桥接
//!
//! gix 是同步 API；GPUI 上下文是异步的（smol）。直接在 GPUI 异步任务里调 gix 会
//! 阻塞 smol worker。本模块用独立线程跑 gix 调用，结果用 oneshot 送回，
//! 调用方 await 即可——既不阻塞 GPUI 线程，也不需要 tokio runtime。
//!
//! 与 `ramag-infra-storage` 用同款模式：redb 也是同步 API，也走 std::thread + oneshot。
//!
//! # 关于 expect
//!
//! `oneshot::Receiver::await` 失败 = 工作线程在送回结果前 panic 了。
//! 我们在线程里 catch_unwind 捕获 panic 并转成错误，所以理论上不会触发；
//! 真发生说明运行时严重异常，让程序崩溃合理
#![allow(clippy::expect_used)]

use std::future::Future;
use std::panic::AssertUnwindSafe;

use futures::FutureExt;
use futures::channel::oneshot;

use ramag_domain::error::{DomainError, Result};

/// 在独立线程跑同步函数 `f`，结果通过 oneshot 送回
///
/// 用法：
/// ```ignore
/// let result = run_blocking(move || gix::open(&path)).await?;
/// ```
pub async fn run_blocking<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        // catch panic：避免 git 操作 panic 时整个线程崩溃，转成 DomainError
        let result = std::panic::catch_unwind(AssertUnwindSafe(f))
            .map_err(|_| DomainError::Other("git operation panicked".into()))
            .and_then(|inner| inner);
        let _ = tx.send(result);
    });
    rx.await
        .expect("git worker thread dropped before sending result")
}

/// `run_blocking` 的 future 化版本（个别场景需要 lazy fold）
pub fn run_blocking_future<F, T>(f: F) -> impl Future<Output = Result<T>>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    run_blocking(f).boxed()
}
