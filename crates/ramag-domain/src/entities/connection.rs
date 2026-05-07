//! 连接配置实体

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 连接唯一标识
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

/// 数据库类型。Hash 派生用于 `ConnectionService` 的 `HashMap<DriverKind, Arc<dyn Driver>>` dispatch
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DriverKind {
    Mysql,
    /// 与 Mysql 共用 SqlBackend 抽象层
    Postgres,
    /// KV 形态，走 KvDriver 而非 Driver
    Redis,
}

impl DriverKind {
    /// 按方言加引号包裹标识符。MySQL 反引号、PG 双引号、Redis 原样
    pub fn quote_identifier(&self, ident: &str) -> String {
        match self {
            DriverKind::Mysql => format!("`{}`", ident.replace('`', "``")),
            DriverKind::Postgres => format!("\"{}\"", ident.replace('"', "\"\"")),
            DriverKind::Redis => ident.to_string(),
        }
    }
}

/// 连接环境色标（dev / staging / prod 视觉区分）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ConnectionColor {
    #[default]
    None,
    Gray,
    Green,
    Blue,
    Yellow,
    Red,
}

impl ConnectionColor {
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

    /// 全部枚举值（UI 选择器用）
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

/// 连接配置。密码运行时明文，落盘前由 storage 层 AES-GCM 加密
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: ConnectionId,
    pub name: String,
    pub driver: DriverKind,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub database: Option<String>,
    pub remark: Option<String>,
    #[serde(default)]
    pub color: ConnectionColor,
}

impl ConnectionConfig {
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

    /// 构造 Redis 连接（username 留空走老版 AUTH，6.0+ ACL 时填用户名）
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

    /// 构造 PostgreSQL 连接。PG 必须指定 database，不能省
    pub fn new_postgres(
        name: impl Into<String>,
        host: impl Into<String>,
        port: u16,
        user: impl Into<String>,
        database: impl Into<String>,
    ) -> Self {
        Self {
            id: ConnectionId::new(),
            name: name.into(),
            driver: DriverKind::Postgres,
            host: host.into(),
            port,
            username: user.into(),
            password: String::new(),
            database: Some(database.into()),
            remark: None,
            color: ConnectionColor::default(),
        }
    }
}
