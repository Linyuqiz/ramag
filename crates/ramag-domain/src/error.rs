//! 领域层统一错误类型。infra 层把 sqlx / redb / gix 等错误转成 DomainError。

use thiserror::Error;

pub type Result<T> = std::result::Result<T, DomainError>;

/// 生产模式（只读保护）拦截写操作时，页面提示的统一文案。
/// 详细拦截信息（具体命令 / 语句）由各 driver 层日志打出，页面只显示这句
pub const READ_ONLY_MESSAGE: &str = "只读模式已开启！";

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

    /// 操作被禁止（生产模式只读保护：拦截写/改/删操作）。
    /// Display 不加前缀——消息体直接作为页面提示文案（见 `READ_ONLY_MESSAGE`）
    #[error("{0}")]
    Forbidden(String),

    /// 兜底
    #[error("未知错误: {0}")]
    Other(String),
}

impl DomainError {
    /// 纯消息体（不含「查询执行失败:」等分类中文前缀），供 CLI 等按自身风格渲染
    pub fn message(&self) -> &str {
        match self {
            DomainError::InvalidConfig(m)
            | DomainError::ConnectionFailed(m)
            | DomainError::QueryFailed(m)
            | DomainError::Storage(m)
            | DomainError::NotFound(m)
            | DomainError::NotImplemented(m)
            | DomainError::Forbidden(m)
            | DomainError::Other(m) => m,
        }
    }

    /// 写操作错误的用户提示：只读拦截（Forbidden）直接用统一文案（不加业务前缀，
    /// 即 `READ_ONLY_MESSAGE`），其余错误加业务前缀便于定位
    pub fn write_hint(&self, prefix: &str) -> String {
        match self {
            DomainError::Forbidden(msg) => msg.clone(),
            other => format!("{prefix}：{other}"),
        }
    }
}
