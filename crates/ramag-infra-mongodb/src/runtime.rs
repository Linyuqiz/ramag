//! MongoDB 专用 tokio runtime（与 SQL / Redis runtime 独立，避免长查询互相挤占）。
//! 桥接形态同 redis::runtime / sql-shared::runtime

use std::future::Future;

use futures::channel::oneshot;
use once_cell::sync::OnceCell;
use tokio::runtime::{Builder, Runtime};

static TOKIO_RUNTIME: OnceCell<Runtime> = OnceCell::new();

/// 惰性初始化。`expect`：runtime 构建失败 = 资源耗尽（不可恢复）
#[allow(clippy::expect_used)]
pub fn tokio_runtime() -> &'static Runtime {
    TOKIO_RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ramag-mongo-tokio")
            .enable_all()
            .build()
            .expect("failed to build mongodb tokio runtime")
    })
}

/// 在 tokio runtime 跑 future，结果经 oneshot 送回。`expect`：sender drop = spawn 内 panic
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
        .expect("mongodb tokio task dropped before sending result")
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
