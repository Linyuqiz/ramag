//! 查询历史记录
//!
//! 每次执行的 SQL 都会被记录下来（不论成功失败），用户可以从历史
//! 面板回看 / 重跑。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ConnectionId;

/// 查询记录唯一 id
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QueryRecordId(pub Uuid);

impl QueryRecordId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for QueryRecordId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for QueryRecordId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 执行结果状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueryStatus {
    Success,
    Failed,
}

/// 一条查询历史
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRecord {
    pub id: QueryRecordId,
    pub connection_id: ConnectionId,
    /// 关联的连接显示名（避免连接被删后历史无法识别）
    pub connection_name: String,
    /// SQL 文本（截断到合理长度，原则上保留完整）
    pub sql: String,
    pub status: QueryStatus,
    /// 执行耗时（毫秒），失败为 0
    pub elapsed_ms: u64,
    /// 受影响 / 返回行数
    pub rows: u64,
    /// 失败时的错误消息
    pub error: Option<String>,
    pub executed_at: DateTime<Utc>,
}

impl QueryRecord {
    pub fn new_success(
        connection_id: ConnectionId,
        connection_name: impl Into<String>,
        sql: impl Into<String>,
        elapsed_ms: u64,
        rows: u64,
    ) -> Self {
        Self {
            id: QueryRecordId::new(),
            connection_id,
            connection_name: connection_name.into(),
            sql: sql.into(),
            status: QueryStatus::Success,
            elapsed_ms,
            rows,
            error: None,
            executed_at: Utc::now(),
        }
    }

    pub fn new_failed(
        connection_id: ConnectionId,
        connection_name: impl Into<String>,
        sql: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            id: QueryRecordId::new(),
            connection_id,
            connection_name: connection_name.into(),
            sql: sql.into(),
            status: QueryStatus::Failed,
            elapsed_ms: 0,
            rows: 0,
            error: Some(error.into()),
            executed_at: Utc::now(),
        }
    }

    /// SQL 单行预览（去除多余空白 + 截断）
    pub fn sql_preview(&self, max_chars: usize) -> String {
        let normalized: String = self
            .sql
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if normalized.chars().count() <= max_chars {
            normalized
        } else {
            let truncated: String = normalized.chars().take(max_chars).collect();
            format!("{truncated}…")
        }
    }
}
