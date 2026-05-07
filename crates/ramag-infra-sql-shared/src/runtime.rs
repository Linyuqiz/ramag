//! tokio↔smol 桥接：GPUI 用 smol，sqlx 强依赖 tokio。
//! 全局 tokio 单例 runtime，`run_in_tokio` 把 future 派给它，结果走 oneshot 回传。
//! `expect`：runtime 构建失败 = 资源耗尽（不可恢复）；oneshot 接收失败 = runtime 异常回收（不应发生）
#![allow(clippy::expect_used)]

use std::future::Future;

use futures::channel::oneshot;
use once_cell::sync::OnceCell;
use tokio::runtime::{Builder, Runtime};

static TOKIO_RUNTIME: OnceCell<Runtime> = OnceCell::new();

/// 惰性初始化全局 tokio runtime
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

/// 在 tokio runtime 跑 future，结果经 oneshot 送回当前 runtime
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
