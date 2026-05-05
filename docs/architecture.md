# Ramag 架构说明

## 设计目标

1. **可扩展**：从一开始就支持多种数据源（当前 MySQL / PostgreSQL / Redis / Git）
2. **可演化**：未来加入新工具（不只是数据库）不需要重构 domain / app 层
3. **可测试**：核心业务逻辑能脱离 GUI 单独测试
4. **可维护**：模块边界清晰，依赖方向单一

## 架构思想

**Clean Architecture 务实版**——保留分层与依赖方向铁律，不强求 4 层、不每个 use case 都拆 Input/Output Port、不引入过度间接层。

### 分层与依赖方向

```
ramag-bin                          ← 入口：依赖注入 + 启动 GPUI
  ├── ramag-tool-dbclient                  ← DB Client 视图（SQL + Redis 共用入口）
  ├── ramag-tool-redis                     ← Redis 专属视图（key 树 / 详情）
  ├── ramag-tool-vcs                       ← VCS（Git）可视化视图
  ├── ramag-ui                             ← Shell + ActivityBar + 主题
  ├── ramag-infra-mysql       impl SqlBackend
  ├── ramag-infra-postgres    impl SqlBackend
  ├── ramag-infra-sql-shared           ← SqlBackend trait + 模板 + tokio runtime
  ├── ramag-infra-redis       impl KvDriver
  ├── ramag-infra-git         impl GitDriver
  ├── ramag-infra-storage     impl Storage（redb + aes-gcm + 钥匙串）
  └── ramag-app                            ← Use Cases + ToolRegistry
        └── ramag-domain                   ← 实体 + traits（无 GPUI / sqlx / redb / redis 依赖）
```

### 铁律

1. **依赖方向单一**：永远向内/向下，禁止反向依赖
2. **Domain 纯净**：仅依赖 serde / thiserror / async-trait / chrono / uuid / futures
3. **接口先于实现**：跨层调用通过 Domain 定义的 trait（`Driver` / `KvDriver` / `GitDriver` / `Storage` / `Tool`）

## Crate 详解

### `ramag-domain`（核心）

**职责**：定义实体 + trait 抽象。

**关键内容**：
- `entities/`：`ConnectionConfig` / `Query` / `QueryResult` / `Schema` / `Table` / `Column` / `RedisValue` / `KeyMeta` / `Branch` / `Commit` / `FileDiff` / 等
- `traits/`：`Driver`（SQL）、`KvDriver`（Redis）、`GitDriver`（Git）、`Storage`、`Tool`
- `error.rs`：统一错误 `DomainError`

**为什么不让 `Driver` 涵盖一切**：SQL / KV / Git 三类后端方法集差异大，强合并会让一侧充斥 NotImplemented，破坏语义清晰度。

### `ramag-app`（应用层）

**职责**：编排 Domain trait 完成业务用例。

**关键内容**：
- `ConnectionService`：SQL 侧 facade，按 `config.driver` 自动分发到 MySQL / Postgres
- `RedisService`：Redis 侧 facade
- `ToolRegistry`：管理已注册的 Tool

### `ramag-infra-sql-shared`（SQL 共享层）

**职责**：MySQL / Postgres / 未来 SQLite 等所有关系型 driver 的唯一抽象层。

**关键内容**：
- `SqlBackend` trait：每个 driver 仅 impl 这一个，方言/取消 SQL/池构造/row 解码全在这里
- `impl_driver_for!` 宏：一行从 `SqlBackend` 生成 Domain `Driver` 实现
- `runtime.rs`：所有 SQL driver 共用的 tokio multi-thread runtime（2 worker）
- `pool.rs`：泛型 `PoolCache<Db>`
- `sql.rs`：多语句切分、LIMIT 注入

**收益**：MySQL / Postgres 各自 lib.rs ~170 行，不重复实现 sqlx 错误映射 / 多语句切分 / cancel handle 等模板。

### `ramag-infra-mysql` / `ramag-infra-postgres`

仅实现 `SqlBackend`。MySQL 用反引号 + `KILL QUERY`，Postgres 用双引号 + `pg_cancel_backend(pid)`、强制连接到具体 db。

### `ramag-infra-redis`

实现 `KvDriver`，封装 redis-rs 的 `aio::ConnectionManager`（自动重连）。

**连接缓存按 `(ConnectionId, db)` 维度**——Redis SELECT 是连接级状态，不能跨 db 复用。

**独立 tokio runtime**：与 SQL 共享会让 SQL 长查询挤占 Redis Pub/Sub 流。

### `ramag-infra-git`

实现 `GitDriver`，底层 [`gix`](https://github.com/Byron/gitoxide)（纯 Rust，性能 2-10× libgit2）。

**同步 → async 桥接**：gix 主要是同步 API，用 `std::thread + futures::oneshot` 派发，**不需要 tokio**——与 Storage 同款模式。

**仓库句柄按 `RepoId` 缓存**；写操作通过 `Mutex<()>` 串行化，避免 `.git/index.lock` 冲突。

### `ramag-infra-storage`

实现 `Storage` trait：连接 CRUD / 查询历史 / 偏好 KV / Git 仓库列表。

**安全**：
- 主密钥由 `keyring` crate 存 macOS 钥匙串（`com.ramag.ramag` / `master_key`），首次启动自动生成
- 密码字段用 `aes-gcm` 加密成 hex 后才落 redb（`EncryptedConnection`）
- 测试通过 `open_with_key(&path, &key)` 入口注入固定密钥，不污染真实钥匙串

**所有数据源共用同一个 Storage 实例**——连接列表统一管理。

### `ramag-tool-dbclient`

DB Client 主视图（SQL + Redis 共用入口）。新建连接表单内通过 driver 选择器决定走 SQL 还是 Redis 路径，按 `DriverKind` 分发到 `SessionEntity::Sql` 或 `SessionEntity::Redis`。

包含：连接列表、连接表单、表树、SQL 编辑器（含补全）、查询面板、结果集表格（行内编辑 / 排序 / 过滤 / 导出）、查询历史。

### `ramag-tool-redis`

Redis 专属视图：DB 切换、Key 树（Trie 命名空间分组 + uniform_list 行级虚拟化）、Key 详情（按 6 类型分发渲染：String / List / Hash / Set / ZSet / Stream）、新建 Key 对话框。

### `ramag-tool-vcs`

Git 客户端，IDEA 风格三栏布局：仓库管理页 / 工作区（Changes / Project Files / Stash）/ 历史日志 / Commit 详情 / Diff 视图（unified + split）/ Blame / Reflog / 冲突编辑器 / Interactive Rebase。

### `ramag-ui`

主壳：`Shell`（左 ActivityBar + 中央 Tool 视图）、`HomeView`（首页）、主题（VSCode 风暗/亮色板）、`RamagAssets`（rust-embed 内嵌 svg + 上游 gpui-component-assets 兜底）。

### `ramag-bin`（主入口）

依赖注入中心：
1. `init_tracing`：默认 `info,ramag=debug`，stderr + 文件双路输出
2. `build_connection_service`：装配 `MysqlDriver` + `PostgresDriver` 进 `HashMap<DriverKind, Arc<dyn Driver>>` + `RedbStorage`
3. `build_redis_service`：装配 `RedisDriver`，复用同一 Storage
4. `build_tool_registry`：注册 `DbClientTool` + `VcsTool`
5. `app.on_reopen`：dock 图标点击/红 X 关窗后重激活时重开主窗口（macOS 习惯）
6. `cx.bind_keys`：注册 `cmd-q` / `cmd-w` / `cmd-enter` 等全局快捷键

## 关键技术决策

### 1) 双 Runtime 桥接

GPUI 内部用 smol，sqlx / redis-rs 强依赖 tokio，**直接调用会 panic**（找不到 tokio reactor）。

| Runtime | 用途 | 来源 |
|---------|------|------|
| smol | UI 事件循环 | GPUI 内部 |
| tokio (SQL) | sqlx 查询 | `ramag-infra-sql-shared::runtime`（MySQL + Postgres + 未来 SQLite 共用，2 worker） |
| tokio (Redis) | redis-rs 操作 | `ramag-infra-redis::runtime`（独立 2 worker） |
| std::thread | redb / gix 同步 API | `Storage` 与 `GitDriver` 各自的 `run_blocking` |

**为什么 SQL/Redis 要分开**：Redis Pub/Sub 长生命周期消费需要独立 worker，否则被 SQL 长查询挤占。

### 2) GPUI / gpui-component 不钉 git rev

钉 rev 会让 ramag 与 gpui-component 各自引用一份 `zed`，类型不互通（`Hsla` 等会被 cargo 当成两个不同类型，编译百余个错）。版本固定靠 `Cargo.lock`。

升级流程：`cargo update -p gpui` + 同步检查 workspace 钉的 `lsp-types` / `ropey` 是否与 gpui-component 内部一致——不一致会因 `InputState` LSP 接口类型不兼容而编译失败。

### 3) `redis` crate features 缺一不可

```toml
features = ["aio", "tokio-comp", "tokio-rustls-comp", "tls-rustls-webpki-roots", "connection-manager"]
```

- 缺 `tokio-rustls-comp`：编译报 `connect_tcp_tls` 缺实现
- 缺 `connection-manager`：`PoolCache` 没有自动重连句柄

### 4) Release Profile 极致优化

`lto = "fat"` + `codegen-units = 1` + `panic = "abort"` + `strip = true`——编译变慢但运行最快、二进制最小。

## 添加新功能的扩展指南

### 加新 Tool（如 JSON 格式化）

1. 新建 `crates/ramag-tool-jsonfmt/`，实现 `Tool` trait
2. 在 `Cargo.toml` 的 `members` 添加该 crate
3. 在 `ramag-bin/src/main.rs` 的 `build_tool_registry` 注册一行
4. 在 `open_main_window` 注册视图工厂到 `Shell::register_tool_view`

**不动 domain / app**——这就是 Clean Architecture 带来的扩展性。

### 加新 SQL 数据库（如 SQLite）

1. 新建 `crates/ramag-infra-sqlite/`，实现 `ramag-infra-sql-shared::SqlBackend` trait
2. crate 末尾写 `ramag_infra_sql_shared::impl_driver_for!(SqliteDriver);` 宏一行
3. 在 `ramag-domain` 的 `DriverKind` 枚举加 `Sqlite` 变体
4. 在 `ramag-bin/main.rs` 的 `build_connection_service` 把 driver 注册进 `HashMap<DriverKind, Arc<dyn Driver>>`

**dbclient 视图层无需改动**——SQL 类共用 `ConnectionSession`。

### 加新 KV 数据库（如 KeyDB / DragonflyDB）

实现 `KvDriver` trait（参考 `ramag-infra-redis`），**不要塞进 `Driver`**——方法集差异大会导致大量 NotImplemented。

## 测试策略

| 层 | 测试类型 | 备注 |
|----|---------|------|
| Domain | 单元测试 | 纯 Rust 逻辑 |
| App | 单元测试 | 编排逻辑 |
| Infra（SQL/Redis） | 集成测试连真实 DB | 缺环境变量自动 skip |
| Infra（Storage / Git） | 单元测试 + tempdir | 不依赖外部服务 |
| UI | 不强测，靠手动验证 | — |

CI（`.github/workflows/ci.yml`）跑 `600-line-gate + fmt-check + check + clippy + test`。集成测试需手动配 `RAMAG_TEST_*` 环境变量启用。

## 参考资料

- [Clean Architecture by Robert Martin](https://blog.cleancoder.com/uncle-bob/2012/08/13/the-clean-architecture.html)
- [zed-industries/zed](https://github.com/zed-industries/zed) — GPUI 框架来源
- [gpui-component](https://github.com/longbridge/gpui-component) — UI 组件库
- [gitoxide](https://github.com/Byron/gitoxide) — 纯 Rust Git 实现
