//! 已打开仓库句柄 + 写操作串行化锁

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use ramag_domain::error::Result;

use crate::runtime::run_blocking;

/// gix::Repository 非 Sync，必须包 Mutex；Arc clone 是 O(1)
pub(crate) struct OpenRepo {
    pub(crate) repo: Arc<Mutex<gix::Repository>>,
    pub(crate) path: PathBuf,
    /// 写操作串行化锁，避免并发触发 `.git/index.lock` 冲突
    pub(crate) write_lock: Arc<Mutex<()>>,
}

/// 写操作 helper：worker 线程内先 lock 再跑。所有写 git index 的方法走这个
pub(crate) async fn run_write_blocking<F, T>(handle: Arc<OpenRepo>, f: F) -> Result<T>
where
    F: FnOnce(&Path) -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    run_blocking(move || {
        let _g = handle.write_lock.lock();
        f(&handle.path)
    })
    .await
}
