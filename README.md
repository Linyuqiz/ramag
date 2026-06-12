# Ramag

Rust + [GPUI](https://github.com/zed-industries/zed) 编写的 macOS 原生桌面工具平台：一个 App 聚合日常开发要用的多个小工具，全部本地运行、数据本地加密存储。

当前内置三个工具，经左侧 ActivityBar 切换：

| 工具 | 说明 |
|---|---|
| **数据库客户端** | MySQL / PostgreSQL / Redis / MongoDB 统一入口，driver 在连接表单内选择 |
| **版本管理** | Git 可视化客户端：仓库管理 / diff / 提交 / 分支 / 推拉合并 |
| **剪贴板** | 剪贴历史：采集 / 搜索筛选 / 全局热键悬浮抽屉快速粘贴，全本地加密 |

## 功能一览

### 数据库客户端

- **连接管理**：连接配置加密落盘（密码 AES-GCM 加密，主密钥存 macOS 钥匙串）、连接测试、颜色标签
- **SQL（MySQL / PostgreSQL）**：库表树（右键重命名 / 清空 / 删除，二次确认）、SQL 编辑器（语法高亮、补全、`⌘⇧F` 格式化、`⌘⇧E` EXPLAIN）、多语句执行、运行中取消、结果集分页 / 单元格编辑 / 导出、DDL 查看、查询历史
- **Redis**：key 树按 `:` 折叠命名空间（5 万+ key 行级虚拟化）、String / Hash / List / Set / ZSet / Stream 全类型查看与编辑、TTL 管理、key 与前缀级删除
- **MongoDB**：database → collection 树、文档表格（嵌套字段扁平化、钻取、编辑、导出）、find / aggregate 等原始命令执行、常用命令示例

### 版本管理（Git）

- 工作区状态自动刷新（文件监听 + 窗口激活触发）、untracked 预览
- diff 分屏对照，按文件后缀全量语法高亮（tree-sitter，35 种语法）
- 提交（amend 保留原 message）、分支 / 标签 / stash、push / pull、merge / rebase / cherry-pick、reflog、blame、冲突三栏编辑、commit graph

### 剪贴板

- 后台采集独立于窗口生死，文本 / 图片（缩略图）历史全本地 AES-GCM 加密存储
- 搜索、按类型筛选、来源应用黑名单、条数 / 天数自动清理
- 全局热键 `⌘⇧V` 唤起屏幕底部悬浮抽屉，选中即粘贴回原应用

## 快速开始

要求：macOS（Apple Silicon / Intel）。Rust 工具链由 `rust-toolchain.toml` 钉死（GPUI 依赖 nightly 特性），首次构建自动安装。

```bash
git clone https://github.com/axemc/ramag.git
cd ramag

make develop        # debug 运行（编译快）
make release        # release 运行（首次 ~2-3 分钟）

make dmg            # 打包当前架构：svg → icns → build → Ramag.app → Ramag.dmg
make dmg-universal  # Intel + Apple Silicon 通用二进制（约 2 倍编译时间）
```

所有常用任务封装在 `Makefile`，直接 `make` 查看完整列表。

## 常用快捷键

| 场景 | 快捷键 |
|---|---|
| SQL / Mongo 查询 | `⌘Enter` 运行 · `⌘⇧Enter` 运行光标处语句 · `⌘T` 新查询 Tab · `⌘W` 关 Tab |
| SQL 编辑 | `⌘⇧F` 格式化 · `⌘⇧E` EXPLAIN · `⌘S` 保存 SQL · `⌘⇧H` 查询历史 · `⌘E` 收起编辑器 |
| VCS | `⌘K` 聚焦提交信息 · `⌘Enter` 提交 · `⌘⇧K` push · `⌘T` pull · `⌘R` 刷新 |
| 剪贴板 | `⌘⇧V` 全局唤起抽屉 · `⌘F` 搜索 · `Enter` 复制 · `↑↓` 选择 |

## 架构

Clean Architecture 务实版，Cargo Workspace 共 17 个 crate，依赖方向严格向内：

```
ramag-bin                 入口：依赖注入 + 启动 GPUI
  ├─ ramag-ui             Shell / ActivityBar / 主题 / 通用对话框
  ├─ ramag-tool-*         工具视图（dbclient / redis / mongodb / vcs / clipboard）
  ├─ ramag-app            Use Cases（ConnectionService / RedisService / MongoService / ClipboardService / ToolRegistry）
  ├─ ramag-infra-*        驱动实现（mysql / postgres / sql-shared / redis / mongodb / git / clipboard / storage）
  └─ ramag-domain         实体 + traits，零 UI / 框架 / 具体技术依赖
```

关键设计：

- **SQL 共享层**（`ramag-infra-sql-shared`）：关系型数据库只需 impl `SqlBackend`（方言 + 解码 + 元数据 SQL），`impl_driver_for!` 宏一行生成 `Driver` 实现；多语句切分、LIMIT 注入、连接池缓存、取消句柄均在共享层，新增 SQLite 等不必重写模板
- **双 runtime 桥接**：GPUI 用 smol，sqlx / redis-rs / mongodb 强依赖 tokio；driver 经 `run_in_tokio` 把 future 派发到独立 tokio runtime，结果用 oneshot 送回
- **凭证安全**：连接配置存 redb，密码字段单独 AES-GCM 加密，主密钥存 macOS 钥匙串，全程不落明文

完整分层说明与「新增数据库 / 新增工具」标准流程见 [docs/architecture.md](docs/architecture.md)。

## 开发

```bash
make check          # cargo check --all-targets（最快的类型检查）
make fmt            # cargo fmt --all
make clippy         # cargo clippy --all-targets -- -D warnings
make test           # cargo test --all
```

工程约束：clippy 将 `unwrap / expect / panic` 标为 warn 且 CI 以 `-D warnings` 门禁；CI 另设单文件 600 行红线。

### 数据库集成测试

`crates/ramag-infra-{mysql,postgres,redis,mongodb}/tests/integration.rs` 连接真实数据库跑端到端流程；对应的一组环境变量缺任一字段即自动 skip，不影响 CI：

```bash
# 以 MySQL 为例（PG / Redis / Mongo 同款前缀：RAMAG_TEST_PG_* / RAMAG_TEST_REDIS_* / RAMAG_TEST_MONGO_*）
export RAMAG_TEST_MYSQL_HOST=127.0.0.1
export RAMAG_TEST_MYSQL_PORT=3306
export RAMAG_TEST_MYSQL_USER=root
export RAMAG_TEST_MYSQL_PASSWORD=...
export RAMAG_TEST_MYSQL_DB=test
cargo test -p ramag-infra-mysql --test integration -- --nocapture
```

### 升级 GPUI

`gpui` 与 `gpui-component` 故意不钉 rev（两者共享 zed 源码，钉版会让 cargo 编译两份 zed、类型不互通），版本锁定靠入库的 `Cargo.lock`。升级用 `cargo update -p gpui`，并同步核对 `lsp-types` / `ropey` 版本与 `gpui-component` 内部一致。

## 数据与日志位置

| 内容 | 路径 |
|---|---|
| 连接配置 / 剪贴板历史（加密 redb） | `~/Library/Application Support/com.ramag.ramag/ramag.redb` |
| 运行日志 | `~/Library/Application Support/com.ramag.ramag/logs/ramag.log` |
| 加密主密钥 | macOS 钥匙串 |

## License

[Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0)
