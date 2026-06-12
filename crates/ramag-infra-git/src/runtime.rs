//! 同步 → async 桥接：gix 同步 API → std::thread + oneshot。与 storage 同款，不需要 tokio。
//! `expect`：oneshot 失败 = 工作线程未送回结果（panic 已被 catch_unwind 捕获，不应触发）
#![allow(clippy::expect_used)]

use std::panic::AssertUnwindSafe;

use futures::channel::oneshot;

use ramag_domain::error::{DomainError, Result};

/// 独立线程跑同步函数，结果经 oneshot 回传
pub async fn run_blocking<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        // panic 转 DomainError，避免线程崩溃
        let result = std::panic::catch_unwind(AssertUnwindSafe(f))
            .map_err(|_| DomainError::Other("git operation panicked".into()))
            .and_then(|inner| inner);
        let _ = tx.send(result);
    });
    rx.await
        .expect("git worker thread dropped before sending result")
}
