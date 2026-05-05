//! 已打开仓库的内部句柄 + 写操作锁机制
//!
//! 拆出来让 `lib.rs` 专注 GitDriver trait impl 框架。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use ramag_domain::error::Result;

use crate::runtime::run_blocking;

/// 已打开仓库的内部句柄
///
/// gix::Repository 不是 Sync，必须包 Mutex；clone 是 Arc 引用计数 +1（O(1)）
pub(crate) struct OpenRepo {
    pub(crate) repo: Arc<Mutex<gix::Repository>>,
    pub(crate) path: PathBuf,
    /// 写操作串行化锁：所有写 git index 的 op（stage/unstage/discard/commit/checkout
    /// /branch/stash/tag/merge/cherry_pick/reset/revert/rebase/patch/remote_admin）
    /// 都在 worker 线程内先 lock 再跑，避免并发触发 `.git/index.lock` 冲突
    pub(crate) write_lock: Arc<Mutex<()>>,
}

/// 写操作 helper：worker 线程内先 lock 写锁，再执行 git 命令
///
/// 闭包接收 repo path 而不是整个 handle，让调用方写起来短小。所有写 git index 的方法
/// 都该走这个，以避免并发触发 `.git/index.lock` 冲突。
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
