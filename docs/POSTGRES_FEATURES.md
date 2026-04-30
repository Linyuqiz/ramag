# PostgreSQL 功能完整矩阵

> 一份"做到什么程度算实际可用"的 PostgreSQL 功能清单，按 16 个维度全展开。
>
> 与 `MYSQL_FEATURES.md` 对齐：相同的优先级标记、相同的章节结构。**仅在每节首段标出 PG 与 MySQL 的关键差异**，避免重复内容。

最后更新：2026-04-29

## 优先级标记

| 标记 | 含义 | 落地版本 |
|------|------|---------|
| 🔴 **L0 必备** | 不做就完全不能用 | v0.3 |
| 🟡 **L1 应有** | 个人日常使用必需 | v0.3 - v0.4 |
| 🟢 **L2 增强** | 生产开发会用到 | v0.4 - v0.5 |
| ⚪ **L3 可选** | DBA / 高级场景，不做也行 | 未定 / 不做 |

> PG 进场较晚（MySQL 已有完整 v0.1 视图），多数 UI 视图直接复用，本文档聚焦 driver 实现和方言差异。

---

## 一、连接管理（16 项）

**与 MySQL 关键差异**：PG 必须连具体 database（不能不指定）；TLS 配置走 `sslmode` 4 档枚举；多了 `search_path` 和 `application_name` 概念。

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| C01 | 主机/端口/用户/密码连接 | 🔴 L0 | 默认端口 5432 |
| C02 | 默认 database 选择（**必填**）| 🔴 L0 | PG 必连具体 db；空字符串拒绝保存 |
| C03 | 字符集协商（client_encoding=UTF8）| 🔴 L0 | sqlx 默认 UTF8 |
| C04 | 连接超时配置 | 🔴 L0 | 防卡死 |
| C05 | 测试连接（"Test" 按钮） | 🔴 L0 | 配置后验证 |
| C06 | 多连接同时打开 | 🟡 L1 | 切换不同库 |
| C07 | 连接池复用（sqlx PgPool） | 🟡 L1 | 同库多查询不重连 |
| C08 | sslmode 4 档（disable/require/verify-ca/verify-full）| 🟡 L1 | 公网连接刚需 |
| C09 | search_path 显示 / 设置 | 🟡 L1 | 默认 schema 解析；连接后 `SHOW search_path` 给提示 |
| C10 | application_name 设置 | 🟡 L1 | 服务端 `pg_stat_activity` 时识别 |
| C11 | autocommit 开关 | 🟡 L1 | 显式事务模式 |
| C12 | 时区设置（SET TIME ZONE） | 🟡 L1 | timestamptz 显示影响 |
| C13 | SSH 隧道连接 | 🟢 L2 | 内网生产库 |
| C14 | 连接颜色标签（dev=绿/prod=红）| 🟢 L2 | 防误操作 |
| C15 | 只读模式（SET TRANSACTION READ ONLY）| 🟢 L2 | 生产保护 |
| C16 | RDS IAM Token 认证 | ⚪ L3 | AWS / Azure 云 |

## 二、元数据查询（17 项）

**与 MySQL 关键差异**：PG 是 database → schema → table 三层；schema 概念在 PG 里是真实的命名空间（不是 db 别名）；多数标准查询走 `information_schema`，少量索引/约束走 `pg_catalog`。

| ID | 功能 | 优先级 | SQL |
|----|------|-------|-----|
| M01 | 列出当前 db 的所有 schemas（排除系统）| 🔴 L0 | `SELECT schema_name FROM information_schema.schemata WHERE schema_name NOT LIKE 'pg_%' AND schema_name != 'information_schema'` |
| M02 | 列出某 schema 下的所有 tables | 🔴 L0 | `... tables WHERE table_schema=?` |
| M03 | 列出某 table 的所有 columns | 🔴 L0 | `... columns WHERE table_schema=? AND table_name=?` |
| M04 | 列详细类型（含长度/精度）| 🔴 L0 | `data_type, character_maximum_length, numeric_precision, numeric_scale` |
| M05 | 显示列注释 | 🟡 L1 | `pg_catalog.col_description(c.oid, attnum)` |
| M06 | 显示列默认值 / NULL 性 | 🔴 L0 | `is_nullable, column_default` |
| M07 | 显示主键 / 唯一约束标识 | 🔴 L0 | join `pg_constraint` 或 `key_column_usage` |
| M08 | 列出 indexes（含 GIN/GIST/BRIN/HASH 类型）| 🟡 L1 | `pg_index` + `pg_am.amname` |
| M09 | 列出 foreign keys | 🟡 L1 | `key_column_usage` + `referential_constraints` |
| M10 | 列出 views | 🟡 L1 | `information_schema.views` |
| M11 | 列出 materialized views | 🟢 L2 | `pg_matviews` |
| M12 | 取建表 DDL | 🟡 L1 | 拼装 `pg_get_*` 函数（无内置 `SHOW CREATE TABLE`）|
| M13 | 表统计（reltuples / 大小）| 🟡 L1 | `pg_class.reltuples` + `pg_total_relation_size()` |
| M14 | 列出 sequences | 🟢 L2 | `information_schema.sequences` + `pg_sequences` |
| M15 | 列出 functions / procedures | 🟢 L2 | `pg_proc` 过滤系统函数 |
| M16 | 列出 triggers | 🟢 L2 | `information_schema.triggers` |
| M17 | 列出 extensions（已安装）| ⚪ L3 | `pg_extension` |

## 三、查询执行（13 项)

**与 MySQL 关键差异**：取消查询走 `SELECT pg_cancel_backend(<pid>)` 而非 `KILL QUERY`；PG 没有 `LAST_INSERT_ID`，用 `RETURNING` 子句返回新主键。

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| Q01 | 执行 SELECT 返回结果集 | 🔴 L0 | 核心 |
| Q02 | 执行 INSERT/UPDATE/DELETE | 🔴 L0 | 返回 affected_rows |
| Q03 | 执行 DDL（CREATE/ALTER/DROP） | 🔴 L0 | — |
| Q04 | 显示执行耗时（毫秒）| 🔴 L0 | "Took 23ms" |
| Q05 | 显示受影响行数 | 🔴 L0 | UPDATE 后看几行被改 |
| Q06 | 显示 RETURNING 子句结果 | 🟡 L1 | 替代 LAST_INSERT_ID |
| Q07 | 显示 NOTICE / WARNING | 🟡 L1 | RAISE NOTICE 输出 |
| Q08 | 多语句分割（按 ; 切分）| 🟡 L1 | 注意 `$$` 包围的函数体 / dollar-quoted string |
| Q09 | 多语句一次执行 | 🟡 L1 | 全部跑或遇错停 |
| Q10 | 取消运行中查询（pg_cancel_backend）| 🟡 L1 | 长查询停止按钮 |
| Q11 | 查询超时配置（statement_timeout）| 🟡 L1 | 全局或单次 |
| Q12 | 流式结果（cursor 游标）| 🟢 L2 | 大结果集 |
| Q13 | 服务端 prepared statement | ⚪ L3 | 性能优化 |

## 四、事务管理（7 项）

**与 MySQL 差异**：默认隔离级 READ COMMITTED；支持 SERIALIZABLE（不只是 SI）；SAVEPOINT 是标准支持。

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| T01 | BEGIN / START TRANSACTION | 🟡 L1 | 显式事务 |
| T02 | COMMIT 按钮 | 🟡 L1 | 提交 |
| T03 | ROLLBACK 按钮 | 🟡 L1 | 回滚 |
| T04 | 当前事务状态显示 | 🟡 L1 | UI 上看是否在事务里 |
| T05 | 长事务警告（>30s 提示）| 🟢 L2 | 防止卡锁 |
| T06 | 隔离级别切换（READ COMMITTED/REPEATABLE READ/SERIALIZABLE）| 🟢 L2 | 标准 4 级 PG 实现 3 级 |
| T07 | SAVEPOINT 支持 | 🟢 L2 | PG 原生支持 |

## 五、类型映射（PostgreSQL → Domain Value，约 19 类型）

**与 MySQL 关键差异**：PG 类型系统更丰富，多了 `jsonb / array / range / interval / inet / uuid` 等；`numeric` 必须保留精度（不转 f64）；`timestamptz` 要保留时区信息。

| ID | PostgreSQL 类型 | Domain Value | 优先级 | 备注 |
|----|----------------|--------------|-------|------|
| TY01 | NULL | `Value::Null` | 🔴 L0 | 必须 |
| TY02 | bool / boolean | `Value::Bool` | 🔴 L0 | true/false |
| TY03 | smallint / integer / bigint | `Value::Int(i64)` | 🔴 L0 | — |
| TY04 | numeric / decimal | `Value::Text`（保留精度）| 🔴 L0 | 不转 f64 损失精度 |
| TY05 | real / double precision | `Value::Float(f64)` | 🔴 L0 | — |
| TY06 | char / varchar / text | `Value::Text(String)` | 🔴 L0 | UTF8 |
| TY07 | bytea | `Value::Bytes` | 🔴 L0 | hex 显示 + 字节数 |
| TY08 | date / timestamp / timestamptz | `Value::DateTime` | 🔴 L0 | timestamptz 带时区显示 |
| TY09 | time / timetz | `Value::Text` | 🟡 L1 | "HH:MM:SS+TZ" |
| TY10 | json / jsonb | `Value::Json` | 🔴 L0 | jsonb pretty 缩进展示 |
| TY11 | uuid | `Value::Text` | 🟡 L1 | "xxxx-xxxx-..." |
| TY12 | array（text[] / int[] 等）| `Value::Text`（"{a,b,c}"）| 🟡 L1 | PG 数组字面量格式 |
| TY13 | enum（用户定义）| `Value::Text` | 🟡 L1 | 字符串值 |
| TY14 | interval | `Value::Text` | 🟡 L1 | "1 year 2 mons" |
| TY15 | money | `Value::Text` | 🟢 L2 | 区域相关，不转 f64 |
| TY16 | inet / cidr / macaddr | `Value::Text` | 🟢 L2 | 网络类型 |
| TY17 | range（int4range/tsrange）| `Value::Text` | 🟢 L2 | "[1,10)" |
| TY18 | hstore（contrib）| `Value::Text` | ⚪ L3 | "k=>v, k2=>v2" |
| TY19 | geometry / geography（PostGIS）| `Value::Text`（WKT）| ⚪ L3 | 空间数据 |

## 六、错误处理（10 项）

**与 MySQL 关键差异**：PG 用 5 字符 SQLSTATE 而非数字；大类前缀有意义（23xx 完整性、42xx 语法权限、28xx 认证）。

| ID | 功能 | 优先级 | SQLSTATE / 说明 |
|----|------|-------|-----|
| E01 | sqlx::Error → DomainError 映射 | 🔴 L0 | 不暴露底层 |
| E02 | 网络错误识别（连不上）| 🔴 L0 | "Cannot connect to host..." |
| E03 | 认证错误（密码错）| 🔴 L0 | `28P01 invalid_password` / `28000 invalid_authorization` |
| E04 | SQL 语法错误 | 🔴 L0 | `42601 syntax_error` |
| E05 | 唯一键冲突 | 🟡 L1 | `23505 unique_violation` |
| E06 | 外键约束 | 🟡 L1 | `23503 foreign_key_violation` |
| E07 | 表不存在 | 🟡 L1 | `42P01 undefined_table` |
| E08 | 字段不存在 | 🟡 L1 | `42703 undefined_column` |
| E09 | 权限不足 | 🟡 L1 | `42501 insufficient_privilege` |
| E10 | 错误日志持久化 | 🟢 L2 | 排查 |

## 七、凭证/安全（8 项）

**与 MySQL 关键差异**：sslmode 4 档（PG 标准）替代 MySQL 的 SSL 配置。

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| S01 | 密码加密存储（aes-gcm）| 🔴 L0 | 复用现有 ramag-infra-storage |
| S02 | macOS 钥匙串保存主密钥 | 🟡 L1 | 同 MySQL |
| S03 | sslmode 选择（disable/require/verify-ca/verify-full）| 🟡 L1 | PG 协议参数 |
| S04 | SSL 证书路径配置（CA/cert/key）| 🟡 L1 | verify-ca/verify-full 模式需要 |
| S05 | SSH 私钥认证（隧道）| 🟢 L2 | 比密码更安全 |
| S06 | 凭证过期/轮换提示 | 🟢 L2 | — |
| S07 | 主密码 / 启动密码 | ⚪ L3 | 整 App 加密 |
| S08 | OS 钥匙串密码 fallback | ⚪ L3 | 多平台 |

## 八、数据修改（结果集 → DML，7 项）

**与 MySQL 关键差异**：PG 行有内部 `ctid` 可用作伪主键定位（但 VACUUM 后会变，不可长期持有）；`ON CONFLICT DO UPDATE` 替代 MySQL 的 `ON DUPLICATE KEY UPDATE`。

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| D01 | 双击单元格编辑 | 🟡 L1 | 直观操作 |
| D02 | 自动生成 UPDATE 语句 | 🟡 L1 | 优先用主键定位，无主键时 ctid（提示风险） |
| D03 | 添加新行 → INSERT | 🟡 L1 | — |
| D04 | 删除行 → DELETE | 🟡 L1 | 二次确认 |
| D05 | 批量提交（事务封装）| 🟡 L1 | 全部成功或全部回滚 |
| D06 | 无主键表 ctid 定位 | 🟢 L2 | 可用但提示不稳 |
| D07 | 复合主键支持 | 🟢 L2 | 多列 WHERE |

## 九、EXPLAIN / 性能（5 项）

**与 MySQL 关键差异**：PG 的 EXPLAIN 远比 MySQL 强 — 文本 / JSON / 树形可选；ANALYZE 给真实执行时间；`pg_stat_statements` 扩展提供慢查询统计。

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| P01 | EXPLAIN 文本展示（PLAN 树） | 🟢 L2 | 直接展示 |
| P02 | EXPLAIN ANALYZE / BUFFERS | 🟢 L2 | 实际执行计划 + 时间 |
| P03 | EXPLAIN (FORMAT JSON) → 树/图 | ⚪ L3 | 实现复杂 |
| P04 | 索引使用情况分析 | ⚪ L3 | 高级 |
| P05 | pg_stat_statements 慢查询 | ⚪ L3 | 需要扩展安装 |

## 十、数据导入导出（8 项）

**与 MySQL 关键差异**：PG 有 `COPY` 命令做高效批量导入导出（流式）；外部工具是 `pg_dump` / `pg_restore`。

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| IO01 | 导出 CSV | 🟡 L1 | 复用现有 export.rs |
| IO02 | 导出 JSON | 🟡 L1 | 同上 |
| IO03 | 复制选中单元格 | 🟡 L1 | Cmd+C |
| IO04 | 导出 INSERT SQL | 🟢 L2 | 含 ON CONFLICT 子句 |
| IO05 | 导出 Markdown 表格 | 🟢 L2 | 写文档用 |
| IO06 | pg_dump 风格 SQL DUMP | ⚪ L3 | 调外部 pg_dump |
| IO07 | CSV 导入（COPY FROM 走客户端流）| ⚪ L3 | 实现复杂 |
| IO08 | SQL 文件批量执行 | 🟢 L2 | 跑迁移脚本 |

## 十一、SQL 处理（8 项）

**与 MySQL 关键差异**：标识符引号是 `"` 而非 \`；多语句分割要识别 `$$` 包围的 dollar-quoted string（函数体里有未转义分号）。

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| SQL01 | 多语句分割（识别 dollar-quoted）| 🟡 L1 | `$$ ... $$` / `$tag$ ... $tag$` 内分号不切 |
| SQL02 | SQL 词法分析 | 🟡 L1 | 关键字 / 标识符 / 字符串 / dollar-quoted |
| SQL03 | LIMIT 自动注入（默认 SELECT 限 500 行）| 🟡 L1 | 防误查全表 |
| SQL04 | SQL 格式化（pretty） | 🟢 L2 | sqlformat-rs 通用方言够用 |
| SQL05 | SQL 关键字大小写规范 | 🟢 L2 | 选项里配 |
| SQL06 | 双引号自动加（标识符冲突时）| 🟢 L2 | 避免 keyword 冲突 |
| SQL07 | SQL 验证（语法预检）| ⚪ L3 | 不连数据库就检查 |
| SQL08 | 完整 SQL parser | ⚪ L3 | 用于补全/重构 |

## 十二、PostgreSQL 方言特性（13 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| DI01 | 双引号标识符 `"column"` | 🔴 L0 | 必备，与 MySQL 反引号区分 |
| DI02 | 注释（-- 和 /\* \*/） | 🔴 L0 | 多语句分割时识别 |
| DI03 | UTF8 字符集（默认） | 🔴 L0 | — |
| DI04 | SERIAL / BIGSERIAL / IDENTITY 列识别 | 🟡 L1 | 列详情显示自增；现代 PG 用 IDENTITY |
| DI05 | RETURNING 子句 | 🟡 L1 | INSERT ... RETURNING id（替代 LAST_INSERT_ID）|
| DI06 | ON CONFLICT (UPSERT) | 🟡 L1 | INSERT ... ON CONFLICT DO UPDATE |
| DI07 | CTE / WITH 子句 | 🟡 L1 | 复杂查询常用 |
| DI08 | 数组字面量 `ARRAY[1,2,3]` / `'{1,2,3}'` | 🟡 L1 | array 类型查询 |
| DI09 | jsonb 操作符（`->` / `->>` / `@>` / `?`）| 🔴 L0 | JSON 数据访问 |
| DI10 | search_path 影响标识符解析 | 🟡 L1 | 同 C09，确认当前 schema 优先 |
| DI11 | 序列（sequences）独立对象 | 🟢 L2 | 列出 + 当前值 + nextval() |
| DI12 | 物化视图（CREATE MATERIALIZED VIEW + REFRESH）| 🟢 L2 | M11 同 |
| DI13 | 自定义类型 / DOMAIN | ⚪ L3 | 罕见 |

## 十三、备份恢复（5 项）

**与 MySQL 关键差异**：调外部 `pg_dump` / `pg_restore`，命令行参数与 mysqldump 不同。

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| BK01 | 单表 DUMP（生成 INSERT）| 🟢 L2 | IO04 同 |
| BK02 | 整库 DUMP | ⚪ L3 | 调 pg_dump |
| BK03 | 从 SQL 文件恢复 | 🟢 L2 | SQL08 同 |
| BK04 | 增量备份（WAL 归档）| ⚪ L3 | 企业向 |
| BK05 | 二进制日志查看（WAL）| ⚪ L3 | DBA 向 |

## 十四、监控/管理（8 项）

**与 MySQL 关键差异**：PG 监控走 `pg_stat_*` 视图族，没有 `SHOW PROCESSLIST` 之类的简单命令。

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| MN01 | pg_stat_activity（谁在跑）| 🟢 L2 | 替代 SHOW PROCESSLIST |
| MN02 | SHOW \<param\> | 🟢 L2 | 看运行时配置 |
| MN03 | pg_stat_database | ⚪ L3 | 数据库统计 |
| MN04 | pg_cancel_backend / pg_terminate_backend | 🟢 L2 | 杀连接 / 杀查询 |
| MN05 | pg_stat_statements 慢查询 | ⚪ L3 | 需要扩展 |
| MN06 | 错误日志查看 | ⚪ L3 | 服务端文件 |
| MN07 | pg_locks 视图 | ⚪ L3 | 锁监控 |
| MN08 | 连接统计 | ⚪ L3 | pg_stat_database.numbackends |

## 十五、用户/权限（5 项）

**与 MySQL 关键差异**：PG 用 ROLE 系统（user 是带 LOGIN 属性的 role），权限粒度更细（schema/table/column/sequence）。

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| US01 | 当前角色权限（`\du` 风格）| 🟢 L2 | `SELECT current_user, session_user` |
| US02 | 创建用户/角色 GUI | ⚪ L3 | 命令行更快 |
| US03 | GRANT/REVOKE GUI | ⚪ L3 | 同上 |
| US04 | 修改密码 | ⚪ L3 | ALTER ROLE |
| US05 | 角色继承可视化 | ⚪ L3 | 复杂 |

## 十六、复制/集群（4 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| RP01 | pg_stat_replication（主从状态）| ⚪ L3 | DBA 专用 |
| RP02 | replication slot 状态 | ⚪ L3 | — |
| RP03 | 主从延迟监控（replay_lag）| ⚪ L3 | — |
| RP04 | 集群拓扑识别（Patroni / Citus）| ⚪ L3 | 企业向 |

---

## 量化总结

| 优先级 | 数量 | 累计 | 落地节点 |
|--------|------|------|---------|
| 🔴 **L0 必备** | 24 | 24 | v0.3 出厂前必须 |
| 🟡 **L1 应有** | 41 | 65 | v0.3 GA 时基本齐 |
| 🟢 **L2 增强** | 31 | 96 | v0.4 - v0.5 |
| ⚪ **L3 可选** | 30 | 126 | 长尾，可不做 |
| **PostgreSQL 整体** | **126 项** | — | — |

## 不同"实际可用"程度的对照

### "凑合能用"（仅 L0 = 24 项）
- 能连
- 能查
- 能看 schema/table/column
- 能基础类型展示（含 jsonb）
- **2-3 周可达**（PG 后入场，driver 镜像 MySQL 写法）

### "个人日常用够"（L0+L1 = 65 项）⭐ **v0.3 目标**
- 多连接、sslmode、连接池
- 元数据完整（含 view/index/FK）
- 多语句、事务、取消查询（pg_cancel_backend）
- 类型完整（含 jsonb / array / uuid / interval）
- 错误友好（SQLSTATE 映射）
- 行内编辑（含 ON CONFLICT）、CSV 导出
- **6-10 周可达**（远少于 MySQL v0.1 的 18-26 周，因为 UI 视图复用）

### "生产开发用够"（L0+L1+L2 = 96 项）⭐ **v0.4 - v0.5 目标**
- SSH 隧道、只读模式
- EXPLAIN / EXPLAIN ANALYZE
- 物化视图 / 序列 / functions / triggers 浏览
- pg_dump 集成
- **+8-12 周（额外）**

### "DBA 级"（全部 L3 = 126 项）⚠️ **不建议做**
- 用户角色管理 GUI
- 复制 / 集群监控
- 慢查询 pg_stat_statements
- PostGIS / 全文搜索 / 自定义类型
- **DataGrip 都没全做齐**

## 给 ramag 的最终建议

**v0.3 锁定 L0+L1 = 65 项**，比 MySQL v0.1 的 75 项略少：PG 后入场，UI 视图（query_panel / result_panel / table_tree / connection_form）90% 复用现有，工作量集中在 PgDriver 实现 + 类型映射 + sslmode + ConnectionService 改多 driver 分发。

**v0.3 GA 后再考虑** L2 的 31 项（EXPLAIN / 物化视图 / 序列等）。L3 的 30 项**明确不做**。

## 与 ROADMAP 的映射

PG 应放在 ROADMAP.md 的 Stage 11（多 DB 支持）展开，建议拆 3 个子 Stage：

| 子阶段 | 范围 | 工作量 |
|--------|------|--------|
| Stage 11.1：核心驱动 | L0 全部（24 项）：连接 + 元数据 + 基础查询 + 基础类型 | 2-3 周 |
| Stage 11.2：方言完整 | L1 主体（~40 项）：sslmode + jsonb 渲染 + RETURNING + ON CONFLICT + array + 取消查询 | 3-4 周 |
| Stage 11.3：增强 | L2 选做（~30 项）：物化视图 + 序列 + EXPLAIN + 行内编辑完善 | 视需求 |

| 优先级 | 在 ROADMAP 的位置 |
|--------|------------------|
| 🔴 L0 | Stage 11.1 |
| 🟡 L1 | Stage 11.1 - 11.2 |
| 🟢 L2 | Stage 11.3（v0.4 / v0.5）|
| ⚪ L3 | 不在 ROADMAP，明确不做 |

## 与 MySQL 的工程量对比

| 维度 | MySQL v0.1 | PostgreSQL v0.3 |
|------|-----------|----------------|
| Driver crate 行数估计 | ~1500 行 | ~1500 行（镜像） |
| 元数据查询差异 | INFORMATION_SCHEMA 标准 | 90% 抄 MySQL，索引方法 / 注释走 pg_catalog |
| 类型数 | 14 类（v0.1 含 13）| 19 类（v0.3 含 14） |
| UI 视图改动 | 全新 | 仅 connection_form 加选项 + DriverKind dispatch |
| ConnectionService 改造 | — | 单 driver → HashMap dispatch（一次性约 50-80 行）|
| 累计工作量 | 18-26 周 | 6-10 周 |

## 结论

| 问题 | 答案 |
|------|------|
| PostgreSQL 整体多少功能？ | **126 项**（按精细化拆分）|
| "实际可用" 至少多少？ | **65 项**（L0+L1）|
| ramag v0.3 目标做多少？ | **65 项 = 实际可用** |
| 开发周期估计？ | **6-10 周**（基于现有 MySQL 视图复用 + 镜像 driver 模式）|
| 必动的核心架构？ | `ConnectionService` 改持有 `HashMap<DriverKind, Arc<dyn Driver>>` 多 driver 分发 |

---

# 附录：PG vs MySQL 差异速查

> 按"对 ramag 实现的影响层"分组，便于实现 PgDriver 时对照 MysqlDriver 决定哪些抄、哪些独立写。

## A1 结构 / 命名空间

| 维度 | MySQL | PostgreSQL | 对 ramag 的影响 |
|------|-------|-----------|----------------|
| 命名层级 | schema = database（**两层**）| catalog → schema → table（**三层**）| `Driver::list_schemas` 语义不同：MySQL 列所有 db；PG 列当前 db 内的所有非系统 schema |
| 必填 database | 可不选（USE 进入）| **必须**连具体 db | `connection_form` 验证：driver=Postgres 时 `database` 必填 |
| 默认 schema 解析 | 当前 USE 的 db | `search_path`（默认 `"$user", public`）| 连接打开后跑 `SHOW search_path` 给 UI 提示 |

## A2 类型系统

| 类型 | MySQL | PostgreSQL | ramag 处理 |
|------|-------|-----------|-----------|
| JSON | `JSON`（5.7+）| `jsonb` + `json` | 复用 `Value::Json` |
| 数组 | 无（用 JSON 替代）| `int[]` / `text[]` 原生 | driver 转 `"{a,b,c}"` → `Value::Text` |
| UUID | `CHAR(36)` / `BINARY(16)` | `uuid` 原生 | driver 转 String → `Value::Text` |
| 时间间隔 | 无 | `interval` 原生 | driver 转 String → `Value::Text` |
| 带时区时间 | `TIMESTAMP` 隐式 UTC | `timestamptz` 显式时区 | 同走 `Value::DateTime` |
| 网络地址 | 无 | `inet` / `cidr` / `macaddr` | driver 转 String → `Value::Text` |
| 范围 | 无 | `int4range` / `tsrange` | driver 转 String → `Value::Text` |
| 高精度数 | `DECIMAL` | `numeric` | 同走 `Value::Text`（保留精度） |

**判定**：PG 5+ 个特有类型全部 fallback `Value::Text`，**`Value` enum 不需扩**。

## A3 SQL 方言

| 维度 | MySQL | PostgreSQL | 对 ramag 的影响 |
|------|-------|-----------|----------------|
| 标识符引号 | `` `col` `` 反引号 | `"col"` 双引号 | `cell_edit_dialog` 按 driver 切换（~10 行） |
| 自增列 | `AUTO_INCREMENT` 列属性 | `SERIAL`/`BIGSERIAL`/`IDENTITY`（实质是 sequence）| 列详情显示文本不同 |
| 取新插主键 | `LAST_INSERT_ID()` | `RETURNING id` 子句 | 用户层差异 |
| UPSERT | `ON DUPLICATE KEY UPDATE` | `ON CONFLICT (...) DO UPDATE` | 用户层差异 |
| 函数体引号 | `'...'` + 转义 | `$$ ... $$` / `$tag$ ... $tag$` dollar-quoted | **多语句分割器**要识别 dollar-quoted（约 30 行增强） |
| 注释 | `--` / `#` / `/* */` | `--` / `/* */`（无 `#`）| 多语句分割器分支微调 |
| 字符串拼接 | `CONCAT()` 优先 | `\|\|` 操作符 | 用户层 |

## A4 连接 / 认证

| 维度 | MySQL | PostgreSQL | 对 ramag 的影响 |
|------|-------|-----------|----------------|
| TLS 配置 | `ssl_mode=DISABLED/PREFERRED/REQUIRED/VERIFY_CA/VERIFY_IDENTITY` | `sslmode=disable/require/verify-ca/verify-full` | `connection_form` 加 PG 专属 sslmode 选择器 |
| 默认端口 | 3306 | 5432 | `connection_form` driver 切换时同步 |
| 用户系统 | `user@host` 二元组 | `ROLE`（user 是带 LOGIN 属性的 role）| UI 仅 username，server 端差异 |
| application_name | 无（用 program_name 隐式）| 连接参数显式 | 可选添加（v0.4） |

## A5 错误码

| 维度 | MySQL | PostgreSQL |
|------|-------|-----------|
| 编码方式 | 数字（如 1062） | **5 字符 SQLSTATE**（如 `23505`）|
| 大类含义 | 无规律 | 前 2 字符为类（**23xxx 完整性 / 42xxx 语法权限 / 28xxx 认证**） |
| 唯一键冲突 | 1062 | 23505 unique_violation |
| 外键违反 | 1452 | 23503 foreign_key_violation |
| 表不存在 | 1146 | 42P01 undefined_table |
| 字段不存在 | 1054 | 42703 undefined_column |
| 权限不足 | 1142 | 42501 insufficient_privilege |
| 认证失败 | 1045 | 28P01 invalid_password / 28000 invalid_authorization |

**对 ramag 的影响**：`ramag-infra-postgres/src/errors.rs` 独立 SQLSTATE→DomainError 映射，**不与 MySQL 共用**（约 30-50 行）。

## A6 运维 / 取消查询

| 维度 | MySQL | PostgreSQL |
|------|-------|-----------|
| 后端 ID 概念 | thread_id（连接级 32-64bit）| backend pid（OS 进程 ID） |
| 拿到 ID | `SELECT CONNECTION_ID()` | `SELECT pg_backend_pid()` |
| 取消运行中查询 | `KILL QUERY <thread_id>` | `SELECT pg_cancel_backend(<pid>)`（保留连接） |
| 强行断连 | `KILL CONNECTION <thread_id>` | `SELECT pg_terminate_backend(<pid>)` |
| 进程列表 | `SHOW PROCESSLIST` | `SELECT * FROM pg_stat_activity` |
| 慢查询 | slow query log 文件 | `pg_stat_statements` 扩展 |

**对 ramag 的影响**：`Driver::execute_cancellable` / `cancel_query` 调 PG 的对应函数（镜像 MySQL 写法）。

## A7 元数据查询

| 项 | MySQL | PostgreSQL |
|----|-------|-----------|
| 列出 schemas | `information_schema.SCHEMATA` | 同（标准），多过滤 `pg_*` 系统 schema |
| 列出 tables | `... TABLES WHERE TABLE_SCHEMA=?` | 同（标准） |
| 列出 columns | `... COLUMNS WHERE TABLE_SCHEMA=? AND TABLE_NAME=?` | 同（标准） |
| 列详细类型 | `COLUMN_TYPE` 一字段（含长度）| 拼 `data_type` + `character_maximum_length` + `numeric_precision/scale` |
| 列注释 | `COLUMN_COMMENT` | `pg_catalog.col_description(c.oid, attnum)` |
| 列出 indexes | `information_schema.STATISTICS` | `pg_index` + `pg_class` + `pg_am`（含索引方法 BTREE/GIN/GIST 等） |
| 取建表 DDL | `SHOW CREATE TABLE` 一句话 | **无等价**！要拼装 `pg_get_*` 函数 |
| 物化视图 | 无 | `pg_matviews` |

**对 ramag 的影响**：PgDriver 元数据查询中 ~70% 抄 MySQL（标准 information_schema），~30% 写 PG 专属（pg_catalog 拼装），合计约 200-300 行。

## A8 差异强度排名（按 ramag 实施优先级）

| 排名 | 差异点 | 影响范围 | 必做？ |
|------|--------|---------|-------|
| 🔴 1 | 三层命名空间 + 必填 db | driver 元数据 + connection_form | L0 |
| 🔴 2 | sslmode 4 档 | connection_form 加 PG 字段 | L0/L1 |
| 🔴 3 | 错误码 SQLSTATE | driver errors.rs（独立映射） | L0 |
| 🟡 4 | 取消查询 API（pg_cancel_backend）| driver execute_cancellable | L1 |
| 🟡 5 | dollar-quoted 多语句切分 | query_tab 多语句分割（按 driver 分支） | L1 |
| 🟡 6 | 双引号标识符 | cell_edit_dialog 生成 UPDATE | L1 |
| 🟡 7 | 5 个 PG 特有类型 fallback Text | driver value 转换层 | L1 |
| 🟢 8 | jsonb 操作符高亮 | 仅 SQL 高亮观感（功能不影响） | L2/不做 |
| 🟢 9 | RETURNING / ON CONFLICT | 用户层（用户写啥 driver 跑啥） | 不做 |
| 🟢 10 | application_name / search_path 显示 | v0.4 增强 | L2 |

**底线**：能日常用 PG 的 80% 价值集中在前 3 条；做完 1-7 即"个人日常用够"（L0+L1）；8-10 是锦上添花。

---

# 附录 B：对照 MySQL 当前实现的最小可用集

> 在 v0.3 完整 L0+L1（65 项）和 MySQL 当前实际落地之间取的"用户感知一致"目标。
>
> 比 65 项多 ~20 项 —— 因为 MySQL 现状里行内编辑 / 导出 / 取消查询 / EXPLAIN 已超额完成。PG 切过来不能"少功能"，否则用户立刻感知。

## B1 MySQL 当前实际落地盘点

```text
ramag-infra-mysql/    1640 行
  driver impl    210  ✓ 全部 10 个 trait 方法
  execute.rs     591  ✓ 含取消查询 / auto_limit / 类型转换
  metadata.rs    258  ✓ schemas/tables/columns/indexes/foreign_keys
  types.rs       259  ✓ MySQL→Value 映射 14 类
  errors.rs      109  ✓ sqlx::Error→DomainError
  pool.rs        127  ✓ 连接池 + thread_id 跟踪
  runtime.rs      86  ✓ tokio↔smol 桥接
```

视图能力（已落地）：
- ✅ 多 tab SQL 编辑器、SQL 高亮（tree-sitter-sequel）
- ✅ Cmd+Enter 执行 / Cmd+Shift+Enter 选中执行
- ✅ 取消运行中查询（`KILL QUERY`）
- ✅ EXPLAIN 文本展示
- ✅ SQL 格式化（sqlformat-rs）
- ✅ auto_limit 自动注入
- ✅ 导出 CSV / JSON / Markdown
- ✅ 查询历史持久化（redb）
- ✅ 单元格行内编辑（含批量事务提交）
- ✅ SQL 补全（关键字 + 表名 + 列名）
- ✅ 表树（schema→table→column + indexes/FK）
- ✅ ConnectionColor 环境标签

## B2 PG 对照必做清单（16 章映射）

| 章 | MySQL 现状项数 | PG 必做项 | 关键差异点 |
|----|-------|----------|-----------|
| 一、连接管理 | 11 | **11** | sslmode 4 档 / search_path / 必填 db / 默认端口 5432 |
| 二、元数据查询 | 11 | **11** | 三层命名 + 索引方法走 pg_catalog + DDL 拼装 |
| 三、查询执行 | 11 | **11** | pg_cancel_backend / RETURNING / dollar-quoted 多语句切分 |
| 四、事务管理 | 4 | **4 (+T07)** | T07 SAVEPOINT 标准支持 |
| 五、类型映射 | 14 类 | **14 类** | 10 类直接映射 + 4 类 fallback Text |
| 六、错误处理 | 9 | **9** | **独立** SQLSTATE 映射（不复用 MySQL 数字码） |
| 七、凭证安全 | 4 | **4** | sslmode + 证书路径（仅 verify-* 模式用） |
| 八、数据修改 | 5 | **5** | 双引号引标识符 / ctid 兜底定位 |
| 九、EXPLAIN | 1（P01 文本）| **1** | PG 输出格式比 MySQL 友好 |
| 十、导入导出 | 4 | **4** | 完全复用 export.rs |
| 十一、SQL 处理 | 4 | **4** | 多语句分割增强：识别 `$$ ... $$` |
| 十二、PG 方言 | (10 MySQL 方言) | **10 PG 方言** | 双引号 / RETURNING / ON CONFLICT / CTE / 数组字面量 / jsonb 操作符 / search_path / SERIAL |
| **合计** | ~78 | **~88 项** | — |

## B3 工作量分配

| 模块 | 工作量 | 说明 |
|------|-------|------|
| `ramag-infra-postgres/` 新 crate | **~1500 行** | 镜像 mysql crate 7 个文件 |
| ↳ driver impl | ~210 | 抄 mysql lib.rs 结构 |
| ↳ execute.rs | ~600 | 改 SQL 方言 + ROW 解析 + cancel 逻辑 |
| ↳ metadata.rs | ~280 | ~70% 抄 mysql 标准 information_schema，~30% 写 pg_catalog |
| ↳ types.rs | ~260 | 14 个标准类型 + 5 个 fallback Text |
| ↳ errors.rs | ~50 | 独立 SQLSTATE 映射 |
| ↳ pool.rs / runtime.rs | ~210 | 几乎照抄 |
| `connection_form.rs` | **~50 行改动** | DRIVER_OPTIONS 加 PG / sslmode 字段 / 默认端口 5432 |
| `dbclient_view.rs` | **~3 行** | match 加 `Postgres => ConnectionSession` |
| `cell_edit_dialog.rs` | **~15 行** | 引号字符按 driver 切换 |
| `query_tab.rs` 多语句分割 | **~30 行** | 识别 dollar-quoted 字符串 |
| `sql_completion.rs` 关键字 | **~10 行** | 加 PG 专属：RETURNING / ON CONFLICT / LATERAL / WITH RECURSIVE |
| `ConnectionService` 多 driver 分发 | **~80 行** | 单 driver → `HashMap<DriverKind, Arc<dyn Driver>>` |
| `DriverKind::Postgres` + `new_postgres` | ~10 | enum 加变体 |
| `main.rs` 装 PgDriver | ~10 | build_connection_service 改返多 driver |
| **合计** | **~1900 行** | 1700 新写 + 200 改动 |

## B4 够用判断（用户场景对照）

| 用户场景 | MySQL 现状 | PG 65 项 L0+L1 | **PG ~88 项对照 MySQL** |
|---------|----------|---------------|------------------------|
| 看 schema/table 结构 | ✅ | ✅ | ✅ |
| 跑 SELECT 看结果 | ✅ | ✅ | ✅ |
| 写 INSERT/UPDATE/DELETE | ✅ | ✅ | ✅ |
| jsonb 列查询展示 | ✅ | ✅ | ✅ |
| 双击单元格改值 | ✅ | ❓ 需做 | ✅ |
| 长查询 Cmd+. 取消 | ✅ | ❓ 需做 | ✅ |
| 导出 CSV / JSON | ✅ | ❓ 需做 | ✅ |
| RETURNING 看新插主键 | n/a | ❓ | ✅ |
| 历史回放上次 SQL | ✅ | ✅ | ✅ |
| 多语句一起执行 | ✅ | ✅ | ✅ |
| EXPLAIN 看执行计划 | ✅ | ❓ | ✅ |
| sslmode 公网连接 | ✅ | ✅ | ✅ |
| SQL 格式化 | ✅ | ✅ | ✅ |
| 表名/字段名补全 | ✅ | ✅ | ✅ |

## B5 推荐策略 & 结论

**做 ~88 项"对照 MySQL 现状"版，而不是 65 项"v0.3 L0+L1"版**。

理由：65 项里不含行内编辑 / 导出 / 取消查询 / EXPLAIN —— 用户从 MySQL 切到 PG 会感知"功能少了"。多做的 ~20 项**基本由既有视图复用得到**（行内编辑、导出 0 改动；取消查询 50 行；EXPLAIN 直接走 execute），边际成本低、用户感知高。

| 项 | 数据 |
|----|------|
| PG 必做项数 | **~88 项** |
| 总工作量 | **~1900 行**（1700 新写 + 200 改动） |
| 周期估计 | **6-8 周** |
| 与 MySQL 体验差距 | **持平** |
| 必动核心架构 | `ConnectionService` 改 `HashMap<DriverKind, Arc<dyn Driver>>` 多 driver 分发 |
| 不做的（明确）| L2/L3 全部：视图/函数/触发器浏览、序列详情、监控、用户权限、复制 |
