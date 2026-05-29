# Ramag

macOS 原生桌面工具平台，定位为「自建版 Raycast / Alfred」。当前版本以数据库客户端 + Git 客户端为主力工具。

> 状态：早期开发中（v0.0.1）。

## 功能

| 工具 | 当前能力 |
|------|---------|
| **DB Client** | MySQL / PostgreSQL：连接管理、Schema/Table 树、SQL 编辑器（关键字 + 表名 + 列名补全 / 语法高亮）、多语句执行、可中断查询、结果集（排序 / 过滤 / 行内编辑 INSERT/UPDATE/DELETE / 导出 CSV/JSON/Markdown/SQL）、查询历史 |
| **Redis** | 连接管理（与 DB Client 共享入口）、DB 切换、Key 树（Trie 命名空间分组 + 虚拟化）、6 种类型详情（String/List/Hash/Set/ZSet/Stream）+ 增删改、TTL 编辑、内存估算 |
| **MongoDB** | 连接管理（与 DB Client 共享入口）、Database/Collection 树、JSON 命令编辑器（语法高亮 + 命令/操作符补全）、文档结果表格（扁平化 + 列/行过滤）、文档 CRUD（新增 / 删除 / 单元格行内编辑 / 导出 JSON·CSV） |
| **VCS（Git）** | IDEA 风格三栏布局、工作区（Changes / Project Files / Stash）、Diff（unified + split）、Commit / Branch / Tag / Stash / Remote 操作、History + 搜索、Commit 详情、Blame、Reflog、Cherry-pick、Merge、Rebase（含 Interactive）、冲突编辑器、Clone |

## 技术栈

- **语言**：Rust nightly（`rust-toolchain.toml` 钉版）
- **UI**：[GPUI](https://github.com/zed-industries/zed)（来自 Zed）+ [gpui-component](https://github.com/longbridge/gpui-component)
- **数据库**：sqlx（MySQL / PostgreSQL）+ redis-rs + mongodb（官方驱动）
- **Git**：[gitoxide](https://github.com/Byron/gitoxide)
- **本地存储**：redb + aes-gcm + macOS 钥匙串

## 快速开始

### 环境要求

- macOS 12+
- Xcode Command Line Tools（`xcode-select --install`）
- Rust（按 `rust-toolchain.toml` 自动安装 nightly）

### 编译运行

```bash
make develop        # debug 模式（编译快，运行慢）
make release        # release 模式（首次 ~2-3 分钟）
```

> ⚠️ 首次 `cargo build` 会拉取并编译 GPUI 全家桶，耗时 30-60 分钟，正常现象。

### 常用命令

```bash
make help            # 列出全部命令
make check           # cargo check
make clippy          # 严格 lint（-D warnings）
make test            # 单元测试
make dmg             # 打包 .dmg（含 svg→icns）
make dmg-universal   # Intel + Apple Silicon 通用二进制
```

## 架构

15 个 crate 的 Cargo Workspace，Clean Architecture 务实版。详见 [`docs/architecture.md`](docs/architecture.md)。

```
ramag-bin                            ← 入口：依赖注入
  ├── ramag-tool-{dbclient,redis,mongodb,vcs}  ← UI 视图
  ├── ramag-ui                               ← Shell + 主题
  ├── ramag-infra-{mysql,postgres}           ← impl SqlBackend
  ├── ramag-infra-sql-shared                 ← SqlBackend + 模板
  ├── ramag-infra-{redis,mongodb,git,storage}← KvDriver / DocDriver / GitDriver / Storage
  └── ramag-app                              ← Use Cases
        └── ramag-domain                     ← 实体 + traits（无外部框架依赖）
```

## 集成测试

数据库类集成测试缺环境变量会自动 skip，不影响 `make test`：

```bash
# MySQL
export RAMAG_TEST_MYSQL_HOST=...
export RAMAG_TEST_MYSQL_PORT=3306
export RAMAG_TEST_MYSQL_USER=...
export RAMAG_TEST_MYSQL_PASSWORD=...
export RAMAG_TEST_MYSQL_DB=...
cargo test -p ramag-infra-mysql --test integration -- --nocapture

# PostgreSQL（DB 必填）
export RAMAG_TEST_PG_HOST=127.0.0.1
export RAMAG_TEST_PG_PORT=5432
export RAMAG_TEST_PG_USER=postgres
export RAMAG_TEST_PG_PASSWORD=...
export RAMAG_TEST_PG_DB=postgres
cargo test -p ramag-infra-postgres --test integration -- --nocapture

# MongoDB（DB 需为 ramag_demo 测试集；USER / PASSWORD 可选）
export RAMAG_TEST_MONGO_HOST=127.0.0.1
export RAMAG_TEST_MONGO_PORT=27017
export RAMAG_TEST_MONGO_DB=ramag_demo
cargo test -p ramag-infra-mongodb --test integration -- --nocapture
```

## License

Apache-2.0
