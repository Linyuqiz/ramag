//! Redis 专用 tokio runtime（与 SQL runtime 独立，避免 SQL 长查询挤占 Pub/Sub）。
//! 桥接形态同 sql-shared::runtime

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
            .thread_name("ramag-redis-tokio")
            .enable_all()
            .build()
            .expect("无法创建 redis 专用 tokio runtime")
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
