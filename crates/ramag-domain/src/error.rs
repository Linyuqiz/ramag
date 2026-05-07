//! 领域层统一错误类型。infra 层把 sqlx / redb / gix 等错误转成 DomainError。

use thiserror::Error;

pub type Result<T> = std::result::Result<T, DomainError>;

#[derive(Debug, Error)]
pub enum DomainError {
    /// 配置无效（字段缺失、URL 解析失败）
    #[error("配置无效: {0}")]
    InvalidConfig(String),

    /// 连接失败（网络 / 认证 / 超时）
    #[error("连接失败: {0}")]
    ConnectionFailed(String),

    /// 查询执行失败（SQL 语法 / 权限 / 超时）
    #[error("查询执行失败: {0}")]
    QueryFailed(String),

    /// 本地存储错误
    #[error("存储错误: {0}")]
    Storage(String),

    /// 实体未找到
    #[error("未找到: {0}")]
    NotFound(String),

    /// 未实现
    #[error("功能尚未实现: {0}")]
    NotImplemented(String),

    /// 兜底
    #[error("未知错误: {0}")]
    Other(String),
}
