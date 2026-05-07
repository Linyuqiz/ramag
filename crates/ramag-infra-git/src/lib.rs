//! Git driver。底层 gix（纯 Rust）+ subprocess git。
//! 同步 API 经 std::thread + oneshot 桥接（不引入 tokio）；按 RepoId 缓存仓库句柄

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

#[derive(Clone, Default)]
pub struct GitDriverImpl {
    /// path → RepoId 去重，同物理仓库不重复打开
    by_path: Arc<DashMap<PathBuf, RepoId>>,
    repos: Arc<DashMap<RepoId, Arc<OpenRepo>>>,
}

impl GitDriverImpl {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn get_repo(&self, id: &RepoId) -> Result<Arc<OpenRepo>> {
        self.repos
            .get(id)
            .map(|r| r.clone())
            .ok_or_else(|| DomainError::InvalidConfig(format!("仓库未打开: {id}")))
    }
}

pub(crate) trait RepoConfigExt {
    fn with_id(self, id: RepoId) -> Self;
}

impl RepoConfigExt for RepoConfig {
    fn with_id(mut self, id: RepoId) -> Self {
        self.id = id;
        self
    }
}
