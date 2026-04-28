# Redis 功能完整矩阵

> 一份"做到什么程度算实际可用"的 Redis 功能清单，按 16 个维度全展开。
>
> 参考：[zedis](https://github.com/vicanso/zedis)、RedisInsight、Another Redis Desktop Manager、Medis、Tiny RDM。

最后更新：2026-04-28

## 优先级标记

| 标记 | 含义 | 落地版本（建议） |
|------|------|----------------|
| 🔴 **L0 必备** | 不做就完全不能用 | v0.4 |
| 🟡 **L1 应有** | 个人日常使用必需 | v0.4 - v0.5 |
| 🟢 **L2 增强** | 生产开发会用到 | v0.5 - v0.6 |
| ⚪ **L3 可选** | DBA / 高级场景，不做也行 | 未定 / 不做 |

> Redis 是项目的**第二个数据库工具**，节奏在 MySQL（v0.1 - v0.3）GA 之后。具体阶段映射见末尾"与 ROADMAP 的映射"。

---

## 一、连接管理（20 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| C01 | host / port 单机连接 | 🔴 L0 | 最基础 |
| C02 | requirepass 密码（AUTH） | 🔴 L0 | 老版本认证 |
| C03 | ACL 用户名 + 密码（6.0+ AUTH user pass） | 🔴 L0 | 现代认证 |
| C04 | SELECT 数据库（0-15） | 🔴 L0 | Redis 多 DB |
| C05 | 测试连接（PING） | 🔴 L0 | 配置后验证 |
| C06 | 连接超时配置 | 🔴 L0 | 防止卡死 |
| C07 | 命令超时配置 | 🟡 L1 | 单命令超时 |
| C08 | 多连接同时打开 | 🟡 L1 | 切换不同实例 |
| C09 | 连接池复用 | 🟡 L1 | 同实例多查询不重连 |
| C10 | 自动重连 | 🟡 L1 | 网络断开后恢复 |
| C11 | CLIENT SETNAME（连接命名） | 🟡 L1 | 服务端可见 |
| C12 | RESP3 协议升级（HELLO） | 🟡 L1 | 解锁新类型 |
| C13 | TLS 连接（rediss://） | 🟡 L1 | 公网/云数据库刚需 |
| C14 | Sentinel 模式（master 名 + 多哨兵地址） | 🟡 L1 | 高可用部署 |
| C15 | Cluster 模式（多节点种子 + 自动重定向） | 🟡 L1 | 分片集群刚需 |
| C16 | SSH 隧道连接 | 🟢 L2 | 内网生产库刚需 |
| C17 | Unix Socket 连接 | 🟢 L2 | 本地高性能 |
| C18 | 连接颜色标签（dev=绿/prod=红） | 🟢 L2 | 防误操作 |
| C19 | 只读模式（写命令拦截） | 🟢 L2 | 生产保护 |
| C20 | IAM / ElastiCache / Memorystore Token 认证 | ⚪ L3 | 云原生鉴权 |

## 二、Key 空间浏览（13 项）

Redis 没有 schema/table 概念，但用 `:` 风格的命名约定（如 `user:1001:profile`）形成事实上的命名空间。

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| K01 | SCAN 分批迭代（cursor） | 🔴 L0 | 替代危险的 KEYS * |
| K02 | MATCH pattern 过滤 | 🔴 L0 | `user:*` |
| K03 | DBSIZE 总数显示 | 🔴 L0 | 顶部计数 |
| K04 | 多 DB 浏览（0-15 切换） | 🔴 L0 | Redis 特有 |
| K05 | 命名空间树（按 `:` 分组折叠） | 🔴 L0 | 上千 Key 不至于平铺 |
| K06 | 大数据集虚拟滚动 | 🔴 L0 | 百万级 Key 不卡 |
| K07 | KEYS * 危险操作拦截 | 🔴 L0 | 默认走 SCAN |
| K08 | SCAN COUNT 调优 | 🟡 L1 | 大库分批粒度 |
| K09 | TYPE 类型过滤 | 🟡 L1 | 只看 hash / list 等 |
| K10 | 树/列表视图切换 | 🟡 L1 | 用户偏好 |
| K11 | 客户端搜索（已加载范围内过滤） | 🟡 L1 | 避免重复 SCAN |
| K12 | Key 类型图标（不同类型不同色） | 🟡 L1 | 快速识别 |
| K13 | Key 收藏夹 / 标签 | 🟢 L2 | 常用 Key 置顶 |

## 三、数据类型支持（16 类型）

Redis 内置 7 种核心类型 + 模块扩展类型。前 5 项必须落地。

| ID | 类型 | 优先级 | 说明 |
|----|------|-------|------|
| T01 | String（含整数、bitmap、bitfield 多态） | 🔴 L0 | SET/GET，INCR 也是 String |
| T02 | List（双端队列） | 🔴 L0 | LPUSH/RPUSH/LRANGE |
| T03 | Hash（field → value） | 🔴 L0 | HSET/HGETALL |
| T04 | Set（无序唯一） | 🔴 L0 | SADD/SMEMBERS |
| T05 | Sorted Set（score 排序） | 🔴 L0 | ZADD/ZRANGE BYSCORE |
| T06 | Stream（消息流，5.0+） | 🟡 L1 | XADD/XRANGE，第七章详述 |
| T07 | RedisJSON（JSON 模块） | 🟡 L1 | JSON.GET / JSON.SET，云上常用 |
| T08 | OBJECT ENCODING 显示 | 🟡 L1 | listpack / quicklist / hashtable 等内部编码 |
| T09 | Bitmap（SETBIT/GETBIT/BITCOUNT） | 🟢 L2 | 用户签到等场景 |
| T10 | HyperLogLog（PFADD/PFCOUNT） | 🟢 L2 | 基数估算 |
| T11 | Geospatial（GEOADD/GEOSEARCH） | 🟢 L2 | 地理位置 |
| T12 | BitField（多位字段） | ⚪ L3 | 位级原子运算 |
| T13 | Bloom / Cuckoo / CMS / TopK（probabilistic 模块） | ⚪ L3 | 模块未必装 |
| T14 | RedisSearch 索引（FT.SEARCH） | ⚪ L3 | 单独大模块 |
| T15 | RedisTimeSeries 数据 | ⚪ L3 | 单独大模块 |
| T16 | RedisGraph 数据 | ⚪ L3 | 已 EOL，不做 |

## 四、值的格式识别与显示（12 项）

Redis 的 String 本质是字节，前端必须做"格式嗅探 + 解码 + 美化"，否则二进制乱码毫无价值。这是 Redis GUI 体验差异化的核心。

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| V01 | UTF-8 文本（默认尝试） | 🔴 L0 | 大多数业务值 |
| V02 | JSON 自动识别 + pretty 折叠 | 🔴 L0 | API 缓存常态 |
| V03 | Hex / 字节预览（无法解码 fallback） | 🔴 L0 | 不读乱码 |
| V04 | 字节大小显示（[N bytes]） | 🔴 L0 | 大值警示 |
| V05 | base64 输入/输出切换 | 🟡 L1 | 二进制粘贴 |
| V06 | MessagePack 自动解码 | 🟡 L1 | 跨语言序列化常用 |
| V07 | Gzip 自动解压 | 🟡 L1 | 压缩缓存场景 |
| V08 | Snappy / LZ4 / Zstd / Brotli 解压 | 🟢 L2 | zedis 同款 |
| V09 | Protobuf 解码（需用户提供 .proto） | 🟢 L2 | 微服务序列化 |
| V10 | 图片预览（PNG/JPEG/WebP/SVG 头识别） | 🟢 L2 | 头像/缩略图缓存 |
| V11 | Java 序列化标识（aced 0005...）警示 | 🟢 L2 | 提示用户解码限制 |
| V12 | 自定义解码脚本（Lua/JS Hook 管道） | ⚪ L3 | 实现复杂，先不做 |

## 五、Key CRUD 与编辑（14 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| D01 | SET String 值（创建/覆写） | 🔴 L0 | 基础 |
| D02 | DEL Key（含二次确认） | 🔴 L0 | 高危必须确认 |
| D03 | List：LPUSH/RPUSH/LPOP/RPOP/LSET | 🔴 L0 | 元素增删改 |
| D04 | Hash：HSET / HDEL（field 编辑） | 🔴 L0 | 表格化编辑 |
| D05 | Set：SADD / SREM | 🔴 L0 | 元素增删 |
| D06 | ZSet：ZADD / ZREM / ZINCRBY（score 编辑） | 🔴 L0 | score 可改 |
| D07 | EXPIRE 设置 TTL | 🔴 L0 | 设置生命周期 |
| D08 | 危险命令拦截（FLUSHDB / FLUSHALL） | 🔴 L0 | 生产事故防线 |
| D09 | RENAME / RENAMENX | 🟡 L1 | Key 重命名 |
| D10 | COPY Key（6.2+） | 🟡 L1 | 跨 DB 复制 |
| D11 | Stream：XADD / XDEL / XTRIM | 🟡 L1 | Stream 写入 |
| D12 | 单元格双击编辑（Hash/ZSet 表格） | 🟡 L1 | 直观操作 |
| D13 | 批量删除多选 Keys | 🟡 L1 | 树面板多选 |
| D14 | 撤销最近一次写（保留旧值回滚） | ⚪ L3 | 实现复杂 |

## 六、TTL 与过期（8 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| X01 | TTL 命令读取（秒） | 🔴 L0 | 显示剩余时间 |
| X02 | EXPIRE 设置秒级 TTL | 🔴 L0 | 最常用 |
| X03 | PERSIST 取消 TTL | 🔴 L0 | 转为永久 |
| X04 | PEXPIRE 毫秒级 TTL | 🟡 L1 | 精细控制 |
| X05 | EXPIREAT / PEXPIREAT 绝对时间 | 🟡 L1 | 业务场景需要 |
| X06 | TTL 倒计时 UI 显示（人性化"剩 3h 22m"） | 🟡 L1 | 阅读友好 |
| X07 | Hash 字段级 TTL（7.4+ HEXPIRE/HTTL/HPERSIST） | 🟢 L2 | 新版本特性 |
| X08 | 即将过期 Key 列表（扫描 + 排序） | 🟢 L2 | 容量规划 |

## 七、命令执行（11 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| CMD01 | 命令行执行（CLI 风格输入框） | 🔴 L0 | 等价 redis-cli |
| CMD02 | 命令历史（向上箭头回溯） | 🔴 L0 | 重跑常用命令 |
| CMD03 | 执行耗时显示 | 🔴 L0 | "Took 3ms" |
| CMD04 | 危险命令默认拦截（FLUSHALL/CONFIG SET/DEBUG） | 🔴 L0 | 需显式 unlock |
| CMD05 | 命令补全（关键字 + 参数提示） | 🟡 L1 | 提升易用性 |
| CMD06 | 命令文档悬浮（参数 / since / complexity） | 🟡 L1 | 内嵌官方文档 |
| CMD07 | 多命令分号分隔批量执行 | 🟡 L1 | 一次跑多条 |
| CMD08 | 命令分组高亮（read/write/admin/dangerous） | 🟡 L1 | 视觉警示 |
| CMD09 | SLOWLOG GET 查看慢命令 | 🟢 L2 | 性能排查 |
| CMD10 | MONITOR 流式查看 | 🟢 L2 | 注意性能开销，需警示 |
| CMD11 | LATENCY 子命令（DOCTOR/HISTORY/RESET） | 🟢 L2 | 延迟分析 |

## 八、RESP 协议与类型映射（14 项）

Redis 通过 RESP（v2 / v3）传输应答。GUI 必须把 RESP 反序列化成统一的 UI 表示。**这是 Driver 层的核心工作**，决定后续渲染上限。

| ID | RESP 类型 | UI 表示 | 优先级 | 备注 |
|----|----------|--------|-------|------|
| R01 | Simple String（`+OK`） | Text | 🔴 L0 | 状态应答 |
| R02 | Error（`-ERR ...`） | DomainError | 🔴 L0 | 错误转域错误 |
| R03 | Integer（`:1000`） | Int | 🔴 L0 | INCR 返回值等 |
| R04 | Bulk String（`$N\r\n...`） | Bytes / Text | 🔴 L0 | 自动 UTF-8 嗅探 |
| R05 | Null Bulk / Null | Null | 🔴 L0 | 不存在 |
| R06 | Array（嵌套） | Vec<Value> | 🔴 L0 | LRANGE / HGETALL |
| R07 | Boolean（RESP3 `#t/#f`） | Bool | 🟡 L1 | 部分新命令 |
| R08 | Double（RESP3 `,3.14`） | Float | 🟡 L1 | ZSCORE 等 |
| R09 | Map（RESP3 `%N`） | Map<String,Value> | 🟡 L1 | HGETALL 直返字典 |
| R10 | Set（RESP3 `~N`） | Set<Value> | 🟡 L1 | SMEMBERS 直返集合 |
| R11 | Verbatim String（`=N`，含格式标识） | Text + 类型标 | 🟢 L2 | INFO 输出等 |
| R12 | Big Number（`(123...456`） | Text（保留精度） | 🟢 L2 | 不能转 i64 |
| R13 | Push（`>N`，异步事件） | 推送通道 | 🟢 L2 | client tracking / Pub/Sub |
| R14 | RESP2 / RESP3 自动协商（HELLO） | — | 🟡 L1 | 启动时升级 |

## 九、发布订阅（7 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| PS01 | SUBSCRIBE 单 channel | 🟡 L1 | 基础订阅 |
| PS02 | PSUBSCRIBE pattern 订阅 | 🟡 L1 | 通配符订阅 |
| PS03 | PUBLISH 发消息 | 🟡 L1 | 测试用 |
| PS04 | 实时消息流面板（时间 + channel + payload） | 🟡 L1 | 调试核心 |
| PS05 | UNSUBSCRIBE 取消 | 🟡 L1 | 释放连接 |
| PS06 | PUBSUB CHANNELS / NUMSUB 查询 | 🟢 L2 | 集群可见性 |
| PS07 | Sharded Pub/Sub（7.0+ SSUBSCRIBE/SPUBLISH） | 🟢 L2 | Cluster 友好 |

## 十、Streams 与消费者组（11 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| ST01 | XADD 添加消息 | 🟡 L1 | 写入 |
| ST02 | XLEN 流长度 | 🟡 L1 | 元数据 |
| ST03 | XRANGE / XREVRANGE 范围读 | 🟡 L1 | 历史浏览 |
| ST04 | XINFO STREAM / GROUPS / CONSUMERS | 🟡 L1 | 元信息 |
| ST05 | XGROUP CREATE / DESTROY | 🟡 L1 | 消费者组管理 |
| ST06 | XTRIM / XDEL 修剪 | 🟡 L1 | 控制大小 |
| ST07 | XACK 确认消费 | 🟡 L1 | 配合组 |
| ST08 | XREAD / XREADGROUP（阻塞读） | 🟢 L2 | 实时消费 |
| ST09 | XPENDING 待确认列表 | 🟢 L2 | 排查未消费 |
| ST10 | XCLAIM / XAUTOCLAIM 转移所有权 | 🟢 L2 | 故障转移 |
| ST11 | Stream 表格化展示（时间戳 + 字段） | 🟡 L1 | UI 渲染 |

## 十一、事务与脚本（10 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| TX01 | MULTI / EXEC 显式事务 | 🟡 L1 | 命令打包 |
| TX02 | DISCARD 回滚队列 | 🟡 L1 | 取消事务 |
| TX03 | EVAL 执行 Lua | 🟡 L1 | 脚本即原子 |
| TX04 | WATCH / UNWATCH 乐观锁 | 🟢 L2 | 并发控制 |
| TX05 | EVALSHA 缓存脚本 | 🟢 L2 | 性能优化 |
| TX06 | SCRIPT LOAD / EXISTS / FLUSH | 🟢 L2 | 脚本管理 |
| TX07 | Lua 脚本编辑器（语法高亮 + 多行） | 🟢 L2 | 编辑体验 |
| TX08 | Functions（7.0+ FCALL / FUNCTION LOAD） | 🟢 L2 | 新机制 |
| TX09 | Function library 浏览 | ⚪ L3 | 高级 |
| TX10 | Lua 脚本调试器（LDB） | ⚪ L3 | 实现复杂 |

## 十二、集群与高可用（12 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| CL01 | 单机模式（standalone） | 🔴 L0 | 默认 |
| CL02 | Sentinel 拓扑（多哨兵 + master 名解析） | 🟡 L1 | 主备切换感知 |
| CL03 | Cluster 拓扑（多种子节点连入） | 🟡 L1 | 分片入口 |
| CL04 | MOVED 重定向自动跟随 | 🟡 L1 | 跨槽位透明 |
| CL05 | ASK 重定向自动跟随 | 🟡 L1 | 迁移期透明 |
| CL06 | 跨分片 SCAN 聚合 | 🟡 L1 | 浏览统一视图 |
| CL07 | CLUSTER NODES / SHARDS 拓扑显示 | 🟢 L2 | 可视化 |
| CL08 | CLUSTER KEYSLOT 计算 | 🟢 L2 | 排查归属 |
| CL09 | 主从拓扑可视化 | 🟢 L2 | 图形展示 |
| CL10 | 副本只读连接（READONLY） | 🟢 L2 | 读写分离 |
| CL11 | 槽位分布热力图 | ⚪ L3 | DBA 向 |
| CL12 | 故障转移触发（CLUSTER FAILOVER / SENTINEL FAILOVER） | ⚪ L3 | 高危操作 |

## 十三、监控与诊断（15 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| M01 | DBSIZE 实时计数 | 🔴 L0 | 顶部一直显示 |
| M02 | INFO 全量信息（分组展示） | 🟡 L1 | 一屏看实例状况 |
| M03 | INFO server / clients / memory / stats / replication / cpu / persistence / keyspace 分组 | 🟡 L1 | 标准九组 |
| M04 | MEMORY USAGE <key> 单 Key 估算 | 🟡 L1 | 排查大 Key |
| M05 | OBJECT ENCODING / IDLETIME / FREQ | 🟡 L1 | 单 Key 详情 |
| M06 | 客户端连接列表（CLIENT LIST） | 🟢 L2 | 谁在连 |
| M07 | 杀客户端（CLIENT KILL） | 🟢 L2 | 应急处理 |
| M08 | SLOWLOG GET / RESET | 🟢 L2 | 慢命令 |
| M09 | LATENCY DOCTOR / HISTORY | 🟢 L2 | 延迟脉冲 |
| M10 | MEMORY STATS / DOCTOR | 🟢 L2 | 内存全景 |
| M11 | BigKey 扫描（仿 redis-cli --bigkeys） | 🟢 L2 | 容量隐患排查 |
| M12 | HotKey 扫描（仿 redis-cli --hotkeys，需 LFU） | 🟢 L2 | 热点识别 |
| M13 | 内存 / QPS 趋势图（采样 INFO 计数） | 🟢 L2 | 时间序列展示 |
| M14 | 命令计数（INFO commandstats） | 🟢 L2 | 调用画像 |
| M15 | 持久化状态（RDB / AOF 最近 lastsave / rewrite） | 🟢 L2 | 备份健康度 |

## 十四、错误处理（12 项）

Redis 错误必须做"语义级"识别再呈现，否则用户只能看到一行英文。

| ID | 错误类型 | 优先级 | 说明 |
|----|---------|-------|------|
| E01 | redis::RedisError → DomainError 映射 | 🔴 L0 | 不暴露底层细节 |
| E02 | 网络错误（连不上 / 超时） | 🔴 L0 | "Cannot connect to host..." |
| E03 | NOAUTH（未认证就发命令） | 🔴 L0 | 提示先 AUTH |
| E04 | WRONGPASS / 认证错误 | 🔴 L0 | 密码错误 |
| E05 | WRONGTYPE（对错误类型操作） | 🔴 L0 | 最常见的意外 |
| E06 | OOM command not allowed（达 maxmemory） | 🟡 L1 | 容量已满 |
| E07 | LOADING（启动加载 RDB 中） | 🟡 L1 | 等服务就绪 |
| E08 | BUSY（脚本/复制阻塞，需 SCRIPT KILL） | 🟡 L1 | 提供操作建议 |
| E09 | READONLY（连到只读副本写） | 🟡 L1 | 切到主节点 |
| E10 | NOSCRIPT（EVALSHA 找不到脚本） | 🟡 L1 | 自动 fallback EVAL |
| E11 | MOVED / ASK（Cluster 拓扑不匹配） | 🟡 L1 | 自动重定向（CL04/05） |
| E12 | 错误日志持久化（与 MySQL 同 redb 表） | 🟢 L2 | 排查历史问题 |

## 十五、凭证与安全（12 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| S01 | 密码加密存储（aes-gcm，复用 storage cipher） | 🔴 L0 | 不能明文 |
| S02 | macOS 钥匙串保管主密钥 | 🟡 L1 | 复用现有机制 |
| S03 | TLS：CA / 客户端 cert / 客户端 key 路径 | 🟡 L1 | 双向认证 |
| S04 | TLS：servername（SNI） | 🟡 L1 | 云数据库要求 |
| S05 | TLS：是否校验证书（insecure 开关） | 🟡 L1 | 内部测试用 |
| S06 | 只读连接模式（拦截 SET/DEL/FLUSH 等写命令） | 🟡 L1 | 配合连接颜色 |
| S07 | 危险命令前端拦截清单（FLUSHALL/CONFIG/DEBUG/SHUTDOWN/KEYS） | 🔴 L0 | 防止手抖 |
| S08 | SSH 私钥认证 | 🟢 L2 | 内网跳板 |
| S09 | SSH 跳板机（多级 forward） | 🟢 L2 | 复杂网络 |
| S10 | ACL WHOAMI / ACL LIST | 🟢 L2 | 我有什么权限 |
| S11 | ACL GETUSER 详细权限 | 🟢 L2 | 排查权限不足 |
| S12 | 主密码 / 启动密码（解锁本地凭证库） | ⚪ L3 | App 全加密 |

## 十六、数据导入导出（10 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| IO01 | 复制单元格（值文本 / Hash field 等） | 🔴 L0 | Cmd+C |
| IO02 | 单 Key 导出 JSON（含类型/TTL/value） | 🟡 L1 | 单条备份 |
| IO03 | 多 Key 批量导出 JSON | 🟢 L2 | 选定 SCAN 结果 |
| IO04 | 导出为 Redis 协议命令脚本（SET/HSET 重放） | 🟢 L2 | 跨实例迁移 |
| IO05 | 导入 JSON 文件 | 🟢 L2 | 恢复单条/批量 |
| IO06 | 导入 Redis 协议命令文件 | 🟢 L2 | 重放 |
| IO07 | 导出 Markdown 表格 | 🟢 L2 | 写文档 / 沟通 |
| IO08 | 跨库迁移（实例 A → 实例 B 复制选定 Keys） | 🟢 L2 | 内置工具 |
| IO09 | RDB 文件解析（离线读） | ⚪ L3 | 实现复杂 |
| IO10 | AOF 文件解析（离线读） | ⚪ L3 | 实现复杂 |

---

## 量化总结

| 优先级 | 数量 | 累计 | 落地节点（建议） |
|--------|------|------|----------------|
| 🔴 **L0 必备** | 53 | 53 | v0.4 出厂前必须 |
| 🟡 **L1 应有** | 74 | 127 | v0.4 GA 时基本齐 |
| 🟢 **L2 增强** | 55 | 182 | v0.5 - v0.6 |
| ⚪ **L3 可选** | 15 | 197 | 长尾，可不做 |
| **Redis 整体** | **197 项** | — | — |

> 注：Redis L0 数量（53）显著高于 MySQL L0（26），原因是 Redis 有 5 个并列的核心数据类型（String/List/Hash/Set/ZSet），每种都需要独立的查看 + 编辑入口，外加 RESP 协议 6 种基础类型的映射，这部分必须一次性铺齐才能"看到东西"；好消息是它们能共用同一套类型分发框架，工程量并不和条目数严格成正比。

## 不同"实际可用"程度对照

### "凑合能用"（仅 L0 = 53 项）
- 能连（含 ACL / TLS）
- 能 SCAN 浏览（命名空间树 + 多 DB 切换）
- 能看 5 大基础类型（String/List/Hash/Set/ZSet）
- 能 CRUD + 设 TTL
- 能 CLI 输命令（含命令历史 + 危险命令拦截）
- JSON 自动 pretty + Hex fallback
- RESP2 全套基础类型映射
- 8-12 周可达
- 像 redis-cli + 文件浏览器

### "个人日常用够"（L0+L1 = 127 项）⭐ **v0.4 目标**
- Sentinel + Cluster 模式（含 MOVED/ASK 自动重定向 + 跨分片 SCAN）
- Streams 浏览（XADD/XRANGE/XINFO/消费者组管理）
- Pub/Sub 实时面板（含 PSUBSCRIBE 模式订阅）
- MULTI / Lua / RedisJSON
- INFO 分组 + 单 Key MEMORY USAGE / OBJECT ENCODING
- MessagePack / Gzip 自动解码 + base64 切换
- 错误语义化（NOAUTH / WRONGTYPE / OOM / BUSY 等 12 类）
- TLS 客户端证书
- RESP3 协议升级 + Map/Set/Boolean 直返
- 友好 TTL 倒计时
- **22-30 周可达**

### "生产开发用够"（L0+L1+L2 = 182 项）⭐ **v0.5 - v0.6 目标**
- SSH 隧道（含跳板）
- BigKey / HotKey / SlowLog / LATENCY 全套诊断
- Cluster 拓扑可视化 + 主从拓扑
- Lua 脚本编辑器 + Functions（7.0+）
- 数据导入导出（JSON / Redis 协议 / 跨库迁移）
- ACL 权限查看（WHOAMI / GETUSER）
- Snappy / LZ4 / Zstd / Brotli 解压
- Protobuf 解码
- **+12-18 周（额外）**

### "DBA 级"（全部 = 197 项，含 L3 共 15 项）⚠️ **不建议做**
- RDB / AOF 离线解析
- Function library / LDB 调试器
- 槽位热力图
- 自定义解码脚本管道
- 故障转移触发
- **RedisInsight 都没全做齐**

## 与 MySQL 文档的差异说明

| 维度 | MySQL | Redis | 差异原因 |
|------|-------|-------|---------|
| 元数据 | 库 / 表 / 列 / 索引 / 外键 | Key 空间（命名空间树）| Redis 无 schema |
| 类型映射 | 13 个 SQL 类型 → Value | 14 个 RESP 类型 → Value | 协议级映射 |
| 查询 | SQL 文本 + 结果集 | 命令 + 多形态应答 | 不存在 SQL parser |
| 事务 | BEGIN/COMMIT/ROLLBACK | MULTI/EXEC + Lua 原子 | Redis 事务不可回滚 |
| 数据修改 | UPDATE/INSERT/DELETE | 类型专属命令 | 按类型分散 |
| 备份恢复 | mysqldump / SQL 文件 | RDB / AOF / 协议脚本 | 完全不同形态 |
| 用户权限 | GRANT/REVOKE | ACL（6.0+）| 现代鉴权模型 |
| 复制集群 | 主从 / GTID / MGR | Sentinel / Cluster | 概念差异大 |
| **Redis 独有** | — | Pub/Sub、Streams、格式嗅探解码 | 必备维度 |

## 与 Ramag 项目的落地映射

> 这一节回答："文档定义的功能，怎么塞进现有架构？"

### 1. Domain 层抽象需要扩展

现有 `ramag-domain/src/traits/driver.rs` 的 `Driver` trait 是**关系型形态**（`list_schemas` / `list_tables` / `list_columns` / `list_indexes` / `list_foreign_keys`），直接拿来给 Redis 用是错配。建议：

- **保留** `Driver` trait 作 SQL 类驱动（MySQL / 未来的 PG）的统一接口
- **新增** `KvDriver` trait（或更通用 `RedisDriver`）放在 `ramag-domain/src/traits/kv_driver.rs`，包含：
  - `test_connection` / `server_version`（与 SQL 驱动同形态）
  - `select_db(db: u8)` / `db_size(db: u8)`
  - `scan(pattern, type_filter, cursor, count) -> (Vec<KeyMeta>, next_cursor)`
  - `get_value(key) -> RedisValue`（按类型自动 dispatch HGETALL / LRANGE / ZRANGE 等）
  - `execute_command(cmd) -> RedisReply`（CLI 通道）
  - `pubsub_subscribe / publish`、`info / memory_usage / slowlog` 等
- **新增** `entities/redis_value.rs`：`RedisValue` 枚举包含 `String / List / Hash / Set / ZSet / Stream / Json` 等 variant；`RedisReply` 表达 RESP 应答（覆盖 RESP2/3 全部 14 种类型）
- `DriverKind::Redis` 已在枚举注释里预留，正式启用即可

### 2. Infra 层新增

新建 `crates/ramag-infra-redis/`，依赖 `redis` crate（async + tokio）：
- 复用现有"双 runtime 桥接"思路（`ramag-infra-mysql/src/runtime.rs` 的 `run_in_tokio`），Redis Driver 同样在 tokio runtime 中跑命令
- 连接池按 `ConnectionId` 缓存，与 MySQL 实现对称
- TLS / SSH 等通过 `redis::ConnectionInfo` 配置

### 3. Tool 层新增

新建 `crates/ramag-tool-redis/`：
- 实现 `Tool` trait（仅元数据：id / name / icon = "redis"）
- 视图按面板拆分（参考 dbclient 的拆法）：`connection_list` / `connection_form` / `key_tree` / `key_detail`（多 tab：String/List/Hash 等渲染器）/ `cli_panel` / `pubsub_panel` / `streams_panel` / `monitor_panel`
- `actions.rs` 集中所有 `#[action(namespace = ramag_redis)]`
- 单文件守 300-600 行红线

### 4. Storage 层无需改动

`ramag-infra-storage` 已经把"密码加密 + 连接 CRUD + 偏好 KV + 历史"做成通用形态，Redis 的连接也走同一套（`ConnectionConfig` 枚举里 driver = Redis 即可）。

### 5. UI 层

`ramag-ui::ActivityBar` 加一个 Redis 图标条目，点击切换到 Redis Tool 视图；`Shell::register_tool_view` 注册 RedisToolView。**不动主壳逻辑**。

## 与 ROADMAP 的映射（建议追加）

当前 `ROADMAP.md` 仅定义了 MySQL 的 Stage 1-13。Redis 作为第二个工具应排在 MySQL v0.3 GA 之后：

| Stage | 阶段 | 周期 | 内容 |
|-------|------|------|------|
| **Stage 14** | Redis 数据层 | 3-4 周 | `KvDriver` trait + `ramag-infra-redis` + 双 runtime 接入 + 集成测试 |
| **Stage 15** | Redis 连接管理 + Key 树 | 3-4 周 | 连接 CRUD（共用 storage）+ SCAN 树 + DB 切换 + 命名空间分组 |
| **Stage 16** | Redis 类型渲染 + CRUD | 4-5 周 | 5 大基础类型 + Streams + JSON 自动 pretty + 编辑面板 |
| **Stage 17** | CLI / Pub/Sub / Streams | 3-4 周 | 命令执行历史 + 实时订阅面板 + Streams 消费者组 |
| **Stage 18** | 监控 / 诊断 / 安全 | 3-4 周 | INFO / SLOWLOG / BigKey / TLS / ACL / 危险命令拦截 |
| **Stage 19** | Cluster / Sentinel / 导入导出 | 3-4 周 | 拓扑 + MOVED 重定向 + JSON 导入导出 + 跨库迁移 |

| 优先级 | 在 ROADMAP 的位置（建议） |
|--------|------------------------|
| 🔴 L0 | Stage 14 - 16 全部 |
| 🟡 L1 | Stage 14 - 18（含基础 UX） |
| 🟢 L2 | Stage 18 - 19（v0.5 / v0.6） |
| ⚪ L3 | 不在 ROADMAP，明确不做 |

## 结论

| 问题 | 答案 |
|------|------|
| Redis 整体多少功能？ | **197 项**（按精细化拆分） |
| "实际可用" 至少多少？ | **127 项**（L0+L1） |
| ramag v0.4 目标做多少？ | **127 项 = 实际可用** |
| 一个人完整做完 197 项要多久？ | **6-9 个月**（v0.4 5-7 个月、+v0.5 / v0.6 累加） |
| 何时启动？ | **MySQL v0.3 GA 之后**（Stage 14 起步） |

## 参考资料

- [zedis](https://github.com/vicanso/zedis) — Rust + GPUI Redis GUI，本项目最直接的对标
- [RedisInsight](https://redis.io/docs/latest/develop/tools/insight/) — 官方 GUI，特性最全
- [Another Redis Desktop Manager](https://github.com/qishibo/AnotherRedisDesktopManager) — 开源参考
- [Medis](https://getmedis.com/) — macOS 原生 Redis GUI
- [Tiny RDM](https://redis.tinycraft.cc/) — 轻量跨平台
- Redis 官方文档：[Commands](https://redis.io/commands/)、[RESP3](https://github.com/redis/redis-specifications/blob/master/protocol/RESP3.md)、[ACL](https://redis.io/docs/management/security/acl/)
