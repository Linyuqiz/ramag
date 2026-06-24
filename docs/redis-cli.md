# Redis 命令行（CLI）设计备忘

> 状态：**待决，未实现**。本文沉淀 2026-06-24 的分析，供日后实现参考。
> 一句话结论：值得加，成本低（后端现成）；展示用 redis-cli 风格文本 transcript；动手前需先拍三个前置决定（见文末）。

## 背景

四个数据库里 Redis 是**唯一没有"命令输入口"**的：

| 库 | 命令输入口 |
|---|---|
| MySQL / PostgreSQL | SQL 编辑器（`query_tab`） |
| MongoDB | JSON `runCommand` 编辑器（`query_tab`） |
| **Redis** | **无**——只有 key 树 + 详情 |

诉求：要不要给 Redis 补一个命令行；若加，异构应答如何展示（Redis 应答形状比 SQL 行列表"不确定性"大得多）。

## 关键事实（已查证，非推测）

1. **后端能力现成**：`KvDriver::execute_command(config, db, argv: Vec<String>) -> Result<RedisValue>`（`crates/ramag-domain/src/traits/kv_driver.rs`），所有写表单（SET / HSET / XADD…）都在用它。重加 CLI **几乎是纯 UI 活，不碰 driver / infra**。
2. **曾经有、已被主动删**：commit `ba2f004 移除 CLI / Pub/Sub`（2026-04-29），含一个 429 行的 `cli_panel.rs` + `pubsub_panel.rs`。删除理由未记录在 commit message。
3. **应答类型已收敛**：`RedisValue`（`crates/ramag-domain/src/entities/redis_value.rs`）是 **12 变体的闭合枚举**（`Nil` / `Text` / `Bytes` / `Int` / `Float` / `Bool` / `List` / `Hash` / `Set` / `ZSet` / `Stream` / `Array`，递归），驱动层已做 RESP→RedisValue 映射。**没有"未知形状"，只有"已知 12 种 + 嵌套"。**
4. **各变体渲染组件现成**：`crates/ramag-tool-redis/src/views/key_detail/{scalar, hash_block, list_block, set_block, zset_block, stream_block}` 已逐变体渲染。

## 决策一：要不要加 —— 倾向加

**支持：**
- 补齐与 SQL / Mongo 的产品一致性（Redis 是唯一缺命令口的）。
- GUI key 浏览器只覆盖常见 CRUD；CLI 覆盖**长尾**：`CONFIG` / `CLUSTER` / `CLIENT` / 调试 / 自定义命令——这些 GUI 永远做不全。
- 重加成本低（后端齐了）。

**前置约束（动手前必须处理）：**
- ⚠️ **生产模式只读保护**（commit `5daaaa2`，"四库后端封死"）：裸 CLI 是这道保护的**天窗**，一条 `FLUSHALL` / `DEL` 即绕过。须先确认 driver 层只读拦截是否覆盖 `execute_command` 的写命令；若否，CLI 自做读写命令分类拦截。**安全红线，不能含糊。**
- **删除原因未知**：重加应是有意识的产品决定，而非"因为别的库有"。

## 决策二：怎么展示 —— 文本 transcript（方案 B）

要点先行：Redis 的"不确定性" = RESP 是递归 sum type，但 `RedisValue` 已把它闭合建模，故"怎么展示" = **渲染一棵 RedisValue 树**，已解决大半。

| 方案 | 做法 | 取舍 |
|---|---|---|
| A 结构化 | 复用 `key_detail` 值渲染器，按变体渲染 | 视觉好；但需把纯值渲染从"key 头部（TTL/类型/键名）"剥离 |
| **B 文本 transcript（推荐）** | 递归 `RedisValue → 缩进文本`，仿 redis-cli（`1) "foo"` / `2) (integer) 42` / 嵌套缩进） | 一个函数吃下所有形状；redis-cli 肌肉记忆；天然支持"输入+输出交错的命令历史滚屏" |
| C 自适应 | 按形状分发（标量→行，扁平 list/set→列表，hash/zset→两列表，嵌套→退回 B） | UX 最好但代码最多，嵌套长尾终究退回 B |

**推荐 B：**
- CLI 定位是**长尾逃生口**，不是第二个结构化查看器（那是 key 浏览器的活）。
- 交互模型与 SQL 本就不同：SQL 是"一次查询换一次结果网格"，CLI 是"连续敲很多条、输入和输出交错成滚屏"，transcript 天然契合。
- KISS / YAGNI：递归格式化函数（约 40 行）覆盖全部 12 变体。将来想让 CLI 兼做漂亮值查看器，再上 C 并复用 `key_detail`。

## 决策三：UI 落点

Redis 会话现为 **tree + detail 两栏**，无 SQL 式 tab / 编辑器，CLI 需安家：
- **底部 console 抽屉**（快捷键 toggle，仿 IDE 终端）—— 推荐，不抢 key 浏览主区。
- 单开一个 console tab。

## 最小可用版（MVP）

底部 console 抽屉 + 输入框 + 文本 transcript（方案 B）+ 生产模式写命令拦截。**后端零改动。**

## 待拍板（实现前需用户确认）

1. 当年删 CLI 是否有意的产品取向？
2. 生产模式：CLI 写命令直接禁，还是给二次确认？
3. UI 落点：底部抽屉 vs console tab？
