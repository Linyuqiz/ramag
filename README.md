# Ramag

> 个人工具平台 / Personal Tool Platform

一个原生、高性能、可扩展的桌面工具集合，**每个工具是独立模块**。第一个工具是 MySQL GUI 客户端。

## 项目愿景

不只是又一个数据库客户端。Ramag 是一个**自建版的 Raycast/Alfred**：
- 🎯 **MySQL GUI 客户端**（v0.1 主力）
- 🛠️ 后续逐步加入"日常小工具"（JSON 格式化、Hash 计算、URL 解析等）
- 🦀 纯 Rust + GPUI，原生性能
- 🧩 模块化架构，新增工具像加插件一样简单

## 架构

采用 **Clean Architecture** 思想（实用版，不教条），Cargo Workspace 多 crate 严格分层：

```
┌────────────────────────────────────────────┐
│  Frameworks (GPUI / sqlx / redb)           │
└─────────────┬──────────────────────────────┘
              │
┌─────────────▼──────────────────────────────┐
│  Infrastructure (实现 Domain trait)         │
│  - ramag-infra-mysql  (sqlx 实现 Driver)   │
│  - ramag-infra-storage (redb 实现 Storage) │
└─────────────┬──────────────────────────────┘
              │
┌─────────────▼──────────────────────────────┐
│  Application (Use Cases)                   │
│  - ramag-app  (ToolRegistry + 用例编排)    │
└─────────────┬──────────────────────────────┘
              │
┌─────────────▼──────────────────────────────┐
│  Domain (核心，纯 Rust)                     │
│  - ramag-domain  (Driver/Storage/Tool)     │
└────────────────────────────────────────────┘

UI:  ramag-ui  (主壳)
Tools: ramag-tool-dbclient  (DB 客户端工具)
Bin:   ramag-bin  (主入口)
```

详见 [docs/architecture.md](./docs/architecture.md)。

## Crate 结构

| Crate | 职责 | 依赖 |
|-------|------|------|
| `ramag-domain` | 核心实体 + trait（无外部依赖）| 无 |
| `ramag-app` | Use Cases + ToolRegistry | domain |
| `ramag-infra-mysql` | MySQL 驱动实现 | domain + sqlx |
| `ramag-infra-storage` | 本地存储实现 | domain + redb |
| `ramag-tool-dbclient` | DB Client 工具实现 | app + ui |
| `ramag-ui` | 主壳 + 共享 UI 基础设施 | app + GPUI |
| `ramag-bin` | 主二进制入口 | 全部 |

## 开发

### 环境要求

- macOS 12+ (Apple Silicon 或 Intel)
- Rust nightly（由 `rust-toolchain.toml` 自动管理）

### 构建

```bash
# 第一次编译会很久（30-60 分钟，下载 + 编译 GPUI 全家桶）
cargo build

# 运行
cargo run -p ramag-bin

# 检查（快速）
cargo check

# 测试
cargo test

# 代码风格
cargo fmt
cargo clippy -- -D warnings
```

## 阶段路线图

| Stage | 目标 | 状态 |
|-------|------|------|
| 0 | Workspace 脚手架 + 主壳 Hello World | **进行中** |
| 1 | MySQL Driver + 本地存储实现 | 待办 |
| 2 | DB Client：连接管理 + 表树 | 待办 |
| 3 | DB Client：编辑器 + 多标签 | 待办 |
| 4 | DB Client：结果集表格 | 待办 |
| 5 | 查询历史 + 收藏 | 待办 |
| 6 | 基础 SQL 补全 | 待办 |

v0.1 目标：4-6 个月。

## 参考项目

- [zed](https://github.com/zed-industries/zed) — GPUI 框架来源
- [gpui-component](https://github.com/longbridge/gpui-component) — UI 组件库
- [zedis](https://github.com/vicanso/zedis) — Redis GUI（架构参考）

## 许可证

Apache 2.0
