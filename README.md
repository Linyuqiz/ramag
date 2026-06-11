# Ramag

macOS 原生桌面工具平台，定位为「自建版 Raycast / Alfred」。纯 Rust + GPUI 打造，当前内置数据库客户端、Git 客户端与剪贴板管理器三大工具。

> 状态：早期开发中（v0.0.1）。所有数据本地存储并加密，不联网、无遥测。

## 功能

| 工具 | 当前能力 |
|------|---------|
| **DB Client（SQL）** | MySQL / PostgreSQL：连接管理、Schema/Table 树、SQL 编辑器（关键字 + 表名 + 列名补全 / 语法高亮）、多语句执行、可中断查询、结果集分页（排序 / 过滤 / 行内 INSERT·UPDATE·DELETE / 导出 CSV·JSON·Markdown·SQL）、查询历史 |
| **Redis** | 连接管理（与 DB Client 共享入口）、DB 切换、Key 树（Trie 命名空间分组 + 虚拟化）、6 种类型详情（String / List / Hash / Set / ZSet / Stream）+ 增删改、TTL 编辑、内存估算 |
| **MongoDB** | Database/Collection 树、JSON 命令编辑器（语法高亮 + 命令/操作符补全）、文档结果表格（扁平化 + 列/行过滤）、文档 CRUD（新增 / 删除 / 单元格行内编辑 / 导出 JSON·CSV） |
| **VCS（Git）** | IDEA 风格三栏布局、工作区（Changes / Project Files / Stash）、Diff（unified + split）、Commit / Branch / Tag / Stash / Remote 操作、History + 搜索、Commit 详情、Blame、Reflog、Cherry-pick、Merge、Rebase（含 Interactive）、冲突编辑器、Clone |
| **剪贴板** | 后台采集（文本 / RTF / 图片 / 文件，链接·颜色自动识别，来源应用标注，连续去重）；历史卡片流 + 搜索 + 类型筛选 + 钉住 + 详情（浏览器打开 / Finder 显示 / 纯文本复制）；全局热键 `cmd-shift-V` 唤起底部悬浮抽屉（仿 Paste，横向大卡片墙 + 中文搜索，双击 / `cmd-1~9` / 回车直接粘贴回原应用，支持全屏 Space）；图片缩略图加速、全本地 AES 加密、密码管理器内容自动跳过、暂停 / 清空 |

## 技术栈

- **语言**：Rust nightly（`rust-toolchain.toml` 钉版）
- **UI**：[GPUI](https://github.com/zed-industries/zed)（来自 Zed）+ [gpui-component](https://github.com/longbridge/gpui-component)
- **数据库**：sqlx（MySQL / PostgreSQL）+ redis-rs + mongodb（官方驱动）
- **Git**：[gitoxide](https://github.com/Byron/gitoxide)
- **剪贴板 / 系统集成**：NSPasteboard / NSWorkspace / CGEvent / Carbon（FFI）+ image（缩略图）
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

### 全局快捷键

| 快捷键 | 作用 |
|--------|------|
| `cmd-shift-V` | 唤起剪贴板悬浮抽屉（全局，全屏应用内也可弹出） |

> 剪贴板抽屉的「自动粘贴回原应用」需要在 **系统设置 › 隐私与安全性 › 辅助功能** 中授权 Ramag；未授权时内容仍会写入剪贴板，可手动 `cmd-V`。

## 架构

17 个 crate 的 Cargo Workspace，Clean Architecture 务实版，依赖方向严格自上而下。详见 [`docs/architecture.md`](docs/architecture.md)。

```
ramag-bin                                  ← 入口：依赖注入 + 启动 GPUI
  ├── ramag-tool-{dbclient,redis,mongodb,vcs,clipboard}  ← UI 视图
  ├── ramag-ui                                         ← Shell + ActivityBar + 主题
  ├── ramag-infra-{mysql,postgres}                     ← impl SqlBackend
  ├── ramag-infra-sql-shared                           ← SqlBackend trait + 模板 + tokio runtime
  ├── ramag-infra-{redis,mongodb,git,clipboard,storage}← KvDriver / DocDriver / GitDriver / ClipboardDriver / Storage
  └── ramag-app                                        ← Use Cases + ToolRegistry
        └── ramag-domain                               ← 实体 + traits（无 UI / 具体技术依赖）
```

**核心抽象**（均定义在 `ramag-domain/src/traits/`，实现在 `infra-*`）：

- `Driver` / `SqlBackend` — SQL 类数据库（MySQL / PostgreSQL，宏生成 Driver）
- `KvDriver` — KV 类数据库（Redis）
- `DocDriver` — 文档类数据库（MongoDB）
- `GitDriver` — Git 操作
- `ClipboardDriver` — 剪贴板采集 / 写回 / 来源应用 / 自动粘贴
- `Storage` — 本地持久化（redb，密码与剪贴图片 AES 加密）

## 集成测试

数据库类集成测试缺环境变量会自动 skip，不影响 `make test`；Git 集成测试对临时仓库跑端到端，随 `make test` 自动执行（机器需装 `git`）。

```bash
# MySQL
export RAMAG_TEST_MYSQL_HOST=... RAMAG_TEST_MYSQL_PORT=3306 \
       RAMAG_TEST_MYSQL_USER=... RAMAG_TEST_MYSQL_PASSWORD=... RAMAG_TEST_MYSQL_DB=...
cargo test -p ramag-infra-mysql --test integration -- --nocapture

# PostgreSQL（DB 必填）
export RAMAG_TEST_PG_HOST=127.0.0.1 RAMAG_TEST_PG_PORT=5432 \
       RAMAG_TEST_PG_USER=postgres RAMAG_TEST_PG_PASSWORD=... RAMAG_TEST_PG_DB=postgres
cargo test -p ramag-infra-postgres --test integration -- --nocapture

# Redis（USERNAME / PASSWORD 可选；测试用 db 15，结尾 FLUSHDB 清场）
export RAMAG_TEST_REDIS_HOST=127.0.0.1 RAMAG_TEST_REDIS_PORT=6379
cargo test -p ramag-infra-redis --test integration -- --nocapture

# MongoDB（USER / PASSWORD 可选）
export RAMAG_TEST_MONGO_HOST=127.0.0.1 RAMAG_TEST_MONGO_PORT=27017 RAMAG_TEST_MONGO_DB=ramag_demo
cargo test -p ramag-infra-mongodb --test integration -- --nocapture
```

## License

Apache-2.0
