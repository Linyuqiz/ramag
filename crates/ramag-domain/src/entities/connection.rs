//! 连接配置相关实体

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 连接的唯一标识符
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConnectionId(pub Uuid);

impl ConnectionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ConnectionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 数据库类型枚举
///
/// v0.1 MySQL，v0.4 起加入 Redis；未来可扩展 PG/SQLite 等
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriverKind {
    Mysql,
    /// Redis（KV 形态，使用 KvDriver trait 而非 Driver trait）
    Redis,
    // 后续阶段添加：
    // Postgres,
    // Sqlite,
}

/// 连接颜色标签：用作环境提示（dev/prod 区分）
///
/// 选 None 时不显示色块；选其他时连接列表 + Tab Bar 都会染色
/// （v0.2 加只读模式后还会用这个色和 readonly 联动加强 prod 警示）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ConnectionColor {
    #[default]
    None,
    /// 灰色（默认/本地）
    Gray,
    /// 绿色（dev / 测试）
    Green,
    /// 蓝色（staging / 预发）
    Blue,
    /// 黄色（共享/QA）
    Yellow,
    /// 红色（prod / 生产，警告）
    Red,
}

impl ConnectionColor {
    /// 用于显示的中文标签
    pub fn label(&self) -> &'static str {
        match self {
            ConnectionColor::None => "无",
            ConnectionColor::Gray => "灰",
            ConnectionColor::Green => "绿（开发）",
            ConnectionColor::Blue => "蓝（预发）",
            ConnectionColor::Yellow => "黄（QA）",
            ConnectionColor::Red => "红（生产）",
        }
    }

    /// 全枚举值，UI 选择器用
    pub fn all() -> &'static [ConnectionColor] {
        &[
            ConnectionColor::None,
            ConnectionColor::Gray,
            ConnectionColor::Green,
            ConnectionColor::Blue,
            ConnectionColor::Yellow,
            ConnectionColor::Red,
        ]
    }
}

/// 连接配置
///
/// 用户在 UI 上填写的连接参数，序列化后存到本地（密码字段单独加密存储）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    /// 唯一标识
    pub id: ConnectionId,
    /// 用户起的名字（如 "midas-dev"）
    pub name: String,
    /// 数据库类型
    pub driver: DriverKind,
    /// 主机
    pub host: String,
    /// 端口
    pub port: u16,
    /// 用户名
    pub username: String,
    /// 密码（运行时使用，存储时加密）
    pub password: String,
    /// 默认数据库（可选）
    pub database: Option<String>,
    /// 备注
    pub remark: Option<String>,
    /// 颜色标签（环境区分）
    #[serde(default)]
    pub color: ConnectionColor,
}

impl ConnectionConfig {
    /// 构造一个 MySQL 连接配置（常用入口）
    pub fn new_mysql(
        name: impl Into<String>,
        host: impl Into<String>,
        port: u16,
        user: impl Into<String>,
    ) -> Self {
        Self {
            id: ConnectionId::new(),
            name: name.into(),
            driver: DriverKind::Mysql,
            host: host.into(),
            port,
            username: user.into(),
            password: String::new(),
            database: None,
            remark: None,
            color: ConnectionColor::default(),
        }
    }

    /// 构造一个 Redis 连接配置（默认端口 6379）
    ///
    /// - `username` 可空字符串：用于 Redis 6.0+ ACL；空时走老版 AUTH（仅密码）
    /// - `database` 默认 None，driver 启动时按 0 号库进入；后续 select_db 可切换
    pub fn new_redis(name: impl Into<String>, host: impl Into<String>, port: u16) -> Self {
        Self {
            id: ConnectionId::new(),
            name: name.into(),
            driver: DriverKind::Redis,
            host: host.into(),
            port,
            username: String::new(),
            password: String::new(),
            database: None,
            remark: None,
            color: ConnectionColor::default(),
        }
    }
}
