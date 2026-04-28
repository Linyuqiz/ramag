# Ramag 架构说明

## 设计目标

1. **可扩展**：从一开始就支持多种数据库（v0.1 仅 MySQL，但能加 PG/Redis）
2. **可演化**：未来加入"日常小工具"（不只是数据库相关）不需要重构
3. **可测试**：核心业务逻辑能脱离 GUI 单独测试
4. **可维护**：模块边界清晰，依赖方向单一

## 选用的架构思想

**Clean Architecture**（Robert Martin 的整洁架构）的精神，但**务实简化**——不强制 4 层、不每个 Use Case 都拆 Input/Output Port、不引入过度的间接层。

### 分层

```
┌─────────────────────────────────────────────┐
│  Frameworks & Drivers (最外层)                │
│  - GPUI / sqlx / redb / fs                  │
└──────────────┬──────────────────────────────┘
               │ 依赖（实现 Domain trait）
┌──────────────▼──────────────────────────────┐
│  Infrastructure                             │
│  - ramag-infra-mysql   (sqlx 实现 Driver)   │
│  - ramag-infra-storage (redb 实现 Storage)  │
└──────────────┬──────────────────────────────┘
               │ 依赖
┌──────────────▼──────────────────────────────┐
│  Application                                │
│  - ramag-app  (ToolRegistry + Use Cases)   │
└──────────────┬──────────────────────────────┘
               │ 依赖
┌──────────────▼──────────────────────────────┐
│  Domain                                     │
│  - ramag-domain  (Entities + Traits)       │
│  - 纯 Rust，无任何外部框架依赖              │
└─────────────────────────────────────────────┘

UI 层（ramag-ui）→ App 层  → Domain 层
                              ↑
                              │
            Infrastructure 层（实现 Domain trait）

Tools 层（ramag-tool-*）→ Domain（Tool trait）
                       → App（被 Registry 注册）

Bin 层（ramag-bin）→ 全部依赖，做依赖注入和 wiring
```

### 铁律

1. **依赖方向单一**：永远向内/向下，禁止反向依赖
2. **Domain 纯净**：domain crate 不能依赖任何业务/UI/具体技术框架
3. **接口先于实现**：所有跨层调用通过 trait（Domain 定义，Infra 实现）

## Crate 详解

### `ramag-domain`（核心）

**职责**：定义业务实体 + trait 抽象。

**依赖**：仅 serde / thiserror / async-trait / chrono / uuid（纯工具类）

**关键内容**：
- `entities/`：核心数据结构
  - `Connection`：连接配置
  - `Query`、`QueryResult`、`Value`：查询和结果
  - `Schema`、`Table`、`Column`：元数据
- `traits/`：抽象接口
  - `Driver`：数据库驱动（实现见 `ramag-infra-mysql`）
  - `Storage`：本地持久化（实现见 `ramag-infra-storage`）
  - `Tool`：工具元数据（实现见 `ramag-tool-*`）
- `error.rs`：统一错误类型 `DomainError`

### `ramag-app`（应用层）

**职责**：编排 Domain trait 完成业务用例。

**依赖**：`ramag-domain` + tokio + parking_lot

**关键内容**：
- `ToolRegistry`：管理已注册的 Tool（线程安全）
- `usecases/`：每个 use case 一个文件
  - `ConnectDatabaseUseCase`
  - `ExecuteQueryUseCase`

### `ramag-infra-mysql`（基础设施 - 数据库）

**职责**：用 sqlx 实现 Domain 的 `Driver` trait。

**依赖**：`ramag-domain` + sqlx + tokio

**关键技术点**：
- sqlx 必须运行在 tokio runtime 内
- 与 GPUI 的 smol runtime 通过 channel 桥接
- 连接池按 `ConnectionId` 缓存

### `ramag-infra-storage`（基础设施 - 存储）

**职责**：用 redb 实现 Domain 的 `Storage` trait。

**依赖**：`ramag-domain` + redb

**关键技术点**：
- 数据库文件路径：`directories::ProjectDirs::from("com", "ramag", "ramag")`
- 密码字段单独 aes-gcm 加密
- redb 同步 API，需用 `tokio::task::spawn_blocking` 包装

### `ramag-tool-dbclient`（DB Client 工具）

**职责**：实现 `Tool` trait，提供 DB 客户端的视图与逻辑。

**依赖**：`ramag-domain`（Stage 0）；后续会加 `ramag-app`、`ramag-ui`

### `ramag-ui`（共享 UI）

**职责**：主壳（Shell）+ 共享 UI 基础设施。

**依赖**：`ramag-domain` + `ramag-app` + GPUI + gpui-component

**关键内容**：
- `Shell`：主窗口视图（左侧 Tool 列表 + 右侧 Tool 视图区）

### `ramag-bin`（主入口）

**职责**：依赖注入 + 启动应用。

**依赖**：所有其他 crate

**流程**：
1. 初始化 tracing
2. 构建 `ToolRegistry`，注册所有 Tool
3. 启动 GPUI App
4. 打开主窗口，挂载 Shell

## 异步策略

### 双 Runtime 共存

| Runtime | 用途 | 谁用 |
|---------|------|------|
| **smol** | UI 事件循环 | GPUI 内部 |
| **tokio** | 数据库 I/O | sqlx |

**原因**：GPUI 选择了 smol（低开销），sqlx 强依赖 tokio（生态丰富）。

### 桥接模式（Stage 1 实施）

```
GPUI (smol)                tokio runtime
─────────────              ────────────────
 用户点击查询  ─→ channel ─→ sqlx 执行查询
 ←──────── channel ──────── 返回结果
 渲染结果
```

实现思路：在 ramag-bin 启动一个独立的 tokio runtime，通过 `tokio::sync::oneshot` 在两个 runtime 之间传递结果。

## 添加新 Tool 的流程（未来）

假设要加一个"JSON 格式化"工具：

1. 新建 `crates/ramag-tool-jsonfmt/`
2. 实现 `Tool` trait：
   ```rust
   pub struct JsonFmtTool { meta: ToolMeta }
   impl Tool for JsonFmtTool { fn meta(&self) -> &ToolMeta { &self.meta } }
   ```
3. 在 `Cargo.toml` workspace `members` 添加该 crate
4. 在 `ramag-bin/src/main.rs` `build_tool_registry` 注册：
   ```rust
   registry.register(Arc::new(JsonFmtTool::new()));
   ```
5. （视图：等 Stage 2 之后引入 ToolView 抽象）

**整个过程不需要修改 `ramag-domain` 或 `ramag-app`** —— 这就是 Clean Architecture 带来的扩展性。

## 添加新数据库支持的流程（未来）

假设要加 PostgreSQL 支持：

1. 新建 `crates/ramag-infra-postgres/`
2. 实现 `Driver` trait：
   ```rust
   pub struct PostgresDriver { /* sqlx PgPool */ }
   #[async_trait]
   impl Driver for PostgresDriver {
       fn name(&self) -> &'static str { "postgres" }
       async fn execute(&self, ...) -> Result<...> { /* 用 sqlx pg */ }
       // ...
   }
   ```
3. 在 `ramag-domain/src/entities/connection.rs` 的 `DriverKind` 枚举添加 `Postgres`
4. 在 `ramag-bin/src/main.rs` 注册：
   ```rust
   driver_registry.register(DriverKind::Postgres, Arc::new(PostgresDriver::new()));
   ```

**Domain/App 不需要修改任何业务逻辑** —— Driver trait 已经把差异封装好。

## 测试策略

| 层 | 测试类型 | 工具 |
|----|---------|------|
| Domain | 单元测试（纯 Rust 逻辑） | `#[test]` |
| App | 单元测试（mock Driver/Storage） | `mockall` |
| Infra | 集成测试（连真实测试 DB） | `testcontainers-rs` |
| UI | 不强测，靠手动验证 | — |

CI 跑前 3 类，保证主干稳定。

## 后续演进

| 时间点 | 引入 |
|-------|------|
| Stage 0（当前）| 主壳 + ToolRegistry 跑通 |
| Stage 1 | 双 runtime 桥接 + MySQL Driver 真实实现 + redb Storage 真实实现 |
| Stage 2 | `ToolView` trait（在 ramag-ui）+ DB Client 视图 |
| Stage 3+ | 编辑器、结果集、历史、补全 |
| 后续 | 多 DB 支持、更多 Tool、自动更新、CI/CD 强化 |

## 参考资料

- [Clean Architecture by Robert Martin](https://blog.cleancoder.com/uncle-bob/2012/08/13/the-clean-architecture.html)
- [zedis](https://github.com/vicanso/zedis) — Redis GUI in Rust + GPUI（架构参考）
- [gpui-component](https://github.com/longbridge/gpui-component) — UI 组件库
