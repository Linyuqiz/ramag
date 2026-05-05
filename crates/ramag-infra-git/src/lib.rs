//! Ramag Git driver
//!
//! 实现 [`ramag_domain::traits::GitDriver`]，底层用 [`gix`]（gitoxide，纯 Rust）。
//!
//! # 设计要点
//!
//! - **同步 → async 桥接**：gix 主要是同步 API，本 crate 的 [`runtime`] 模块用
//!   `std::thread + futures::oneshot` 把同步调用派发到独立线程，结果用 oneshot 送回，
//!   让 GPUI 异步任务能 await。**不需要 tokio runtime**（与 `ramag-infra-storage` 同款模式）
//! - **仓库句柄缓存**：按 [`RepoId`](ramag_domain::entities::RepoId) 索引缓存
//!   [`gix::Repository`] 句柄，避免每次操作重新打开仓库
//! - **错误映射**：把 gix 各模块的错误聚合成 [`ramag_domain::error::DomainError`]，
//!   附带中文消息便于 UI 展示

pub mod blame;
pub mod cherry_pick;
pub mod clone;
pub mod commit_files;
pub mod commit_op;
pub mod conflict_content;
pub mod conflict_ops;
pub mod diff;
pub(crate) mod driver;
pub mod errors;
pub mod git_cmd;
mod handle;
pub mod history_ops;
pub mod log;
pub mod merge;
pub mod patch;
pub mod rebase;
pub mod rebase_interactive;
pub mod reflog;
pub mod remote;
pub mod runtime;
pub mod stash;
pub mod status;
pub mod tag;
pub mod work_ops;

use std::path::PathBuf;
use std::sync::Arc;

use dashmap::DashMap;

use ramag_domain::entities::{RepoConfig, RepoId};
use ramag_domain::error::{DomainError, Result};

use crate::handle::OpenRepo;

/// Git 驱动主结构
#[derive(Clone, Default)]
pub struct GitDriverImpl {
    /// path → OpenRepo（按物理路径去重，避免同一仓库被多次打开）
    by_path: Arc<DashMap<PathBuf, RepoId>>,
    /// RepoId → OpenRepo
    repos: Arc<DashMap<RepoId, Arc<OpenRepo>>>,
}

impl GitDriverImpl {
    pub fn new() -> Self {
        Self::default()
    }

    /// 内部：取已打开仓库句柄；没打开就报错
    pub(crate) fn get_repo(&self, id: &RepoId) -> Result<Arc<OpenRepo>> {
        self.repos
            .get(id)
            .map(|r| r.clone())
            .ok_or_else(|| DomainError::InvalidConfig(format!("仓库未打开: {id}")))
    }
}

/// 给 RepoConfig 提供从外部注入 RepoId 的便利构造方法
pub(crate) trait RepoConfigExt {
    fn with_id(self, id: RepoId) -> Self;
}

impl RepoConfigExt for RepoConfig {
    fn with_id(mut self, id: RepoId) -> Self {
        self.id = id;
        self
    }
}
