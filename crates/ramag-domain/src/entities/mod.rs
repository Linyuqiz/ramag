//! 领域实体：纯 Rust 数据结构 + serde。

pub mod clipboard;
pub mod connection;
pub mod git;
pub mod history;
pub mod mongo;
pub mod query;
pub mod redis_keyspace;
pub mod redis_value;
pub mod schema;

pub use clipboard::{
    CapturedClip, ClipId, ClipItem, ClipKind, ClipSource, ClipboardSettings, classify_text,
    fnv1a_hash, make_preview, parse_hex_color,
};
pub use connection::{ConnectionConfig, ConnectionId, DriverKind};
pub use git::{
    BlameLine, Branch, BranchKind, Commit, CommitId, ConflictContent, DiffKind, DiffLine,
    DiffLineKind, FileChangeKind, FileDiff, FileStatus, Hunk, LogOptions, RebaseAction, RebaseTodo,
    ReflogEntry, Remote, RepoConfig, RepoId, RepoOperation, ResetKind, Signature, Stash, StashId,
    Tag, TagKind, WorkingTreeStatus,
};
pub use history::{QueryRecord, QueryRecordId, QueryStatus};
pub use mongo::{
    MongoCollection, MongoCollectionStats, MongoDatabase, MongoDocument, MongoIndex,
    MongoQueryResult, MongoQuerySpec,
};
pub use query::{Query, QueryResult, Row, Value, Warning};
pub use redis_keyspace::{KeyMeta, RedisType, ScanResult};
pub use redis_value::{RedisValue, StreamEntry};
pub use schema::{Column, ColumnKind, ColumnType, ForeignKey, Index, Schema, Table};
