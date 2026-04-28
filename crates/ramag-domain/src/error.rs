//! 领域层错误定义
//!
//! 所有 trait 方法返回 `Result<T, DomainError>`，infra 层实现时把具体错误
//! （sqlx::Error、redb::Error 等）转换成 DomainError。

use thiserror::Error;

pub type Result<T> = std::result::Result<T, DomainError>;

/// 领域层统一错误类型
///
/// 上层（app、ui）只感知到 DomainError，不需要了解底层实现细节
#[derive(Debug, Error)]
pub enum DomainError {
    /// 配置无效（连接配置缺失字段、URL 解析失败等）
    #[error("配置无效: {0}")]
    InvalidConfig(String),

    /// 连接失败（网络、认证、超时）
    #[error("连接失败: {0}")]
    ConnectionFailed(String),

    /// 查询执行失败（SQL 语法、权限、超时）
    #[error("查询执行失败: {0}")]
    QueryFailed(String),

    /// 数据存储错误（本地文件、redb）
    #[error("存储错误: {0}")]
    Storage(String),

    /// 实体未找到（指定 id 的连接 / Tool 不存在）
    #[error("未找到: {0}")]
    NotFound(String),

    /// 未实现（Stage 0 阶段大量 trait 方法都是这个）
    #[error("功能尚未实现: {0}")]
    NotImplemented(String),

    /// 其他兜底错误
    #[error("未知错误: {0}")]
    Other(String),
}
