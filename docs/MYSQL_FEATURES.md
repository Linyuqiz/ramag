# MySQL 功能完整矩阵

> 一份"做到什么程度算实际可用"的 MySQL 功能清单，按 16 个维度全展开。

最后更新：2026-04-26

## 优先级标记

| 标记 | 含义 | 落地版本 |
|------|------|---------|
| 🔴 **L0 必备** | 不做就完全不能用 | v0.1 |
| 🟡 **L1 应有** | 个人日常使用必需 | v0.1 - v0.2 |
| 🟢 **L2 增强** | 生产开发会用到 | v0.2 - v0.3 |
| ⚪ **L3 可选** | DBA / 高级场景，不做也行 | 未定 / 不做 |

---

## 一、连接管理（16 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| C01 | 主机/端口/用户/密码连接 | 🔴 L0 | 最基础 |
| C02 | 默认数据库选择 | 🔴 L0 | USE 进入 |
| C03 | 字符集协商（utf8mb4 默认）| 🔴 L0 | 否则中文乱码 |
| C04 | 连接超时配置 | 🔴 L0 | 防止卡死 |
| C05 | 测试连接（"Test" 按钮） | 🔴 L0 | 配置后验证 |
| C06 | 多连接同时打开 | 🟡 L1 | 切换不同库 |
| C07 | 连接池复用 | 🟡 L1 | 同库多查询不重连 |
| C08 | 时区设置 | 🟡 L1 | 'time_zone' 参数 |
| C09 | autocommit 开关 | 🟡 L1 | 显式事务模式 |
| C10 | 自动重连 | 🟡 L1 | 网络断开后恢复 |
| C11 | SSL/TLS 加密连接 | 🟡 L1 | 公网连接刚需 |
| C12 | SSH 隧道连接 | 🟢 L2 | 内网生产库刚需 |
| C13 | 连接颜色标签（dev=绿/prod=红）| 🟢 L2 | 防误操作 |
| C14 | 只读模式（防写）| 🟢 L2 | 生产保护 |
| C15 | 多级 SSH 跳板 | ⚪ L3 | 企业内网 |
| C16 | IAM / 云数据库 Token 认证 | ⚪ L3 | AWS RDS / 阿里云 |

## 二、元数据查询（17 项）

通过 `INFORMATION_SCHEMA` / `SHOW` 命令读取数据库结构信息。

| ID | 功能 | 优先级 | SQL |
|----|------|-------|-----|
| M01 | 列出所有 schemas | 🔴 L0 | `SELECT SCHEMA_NAME FROM information_schema.SCHEMATA` |
| M02 | 列出某 schema 的所有 tables | 🔴 L0 | `... TABLES WHERE TABLE_SCHEMA=?` |
| M03 | 列出某 table 的所有 columns | 🔴 L0 | `... COLUMNS WHERE TABLE_SCHEMA=? AND TABLE_NAME=?` |
| M04 | 显示列的详细类型（含长度/精度）| 🔴 L0 | `COLUMN_TYPE` |
| M05 | 显示列的注释 | 🟡 L1 | `COLUMN_COMMENT` |
| M06 | 显示列的默认值 / NULL 性 | 🔴 L0 | `IS_NULLABLE`, `COLUMN_DEFAULT` |
| M07 | 显示主键标识 | 🔴 L0 | `COLUMN_KEY = 'PRI'` |
| M08 | 列出 indexes | 🟡 L1 | `... STATISTICS WHERE TABLE_NAME=?` |
| M09 | 列出 foreign keys | 🟡 L1 | `KEY_COLUMN_USAGE` |
| M10 | 列出 views | 🟡 L1 | `VIEWS` 表 |
| M11 | SHOW CREATE TABLE | 🟡 L1 | 取建表 DDL |
| M12 | 表统计（估算行数、大小）| 🟡 L1 | `... TABLES.TABLE_ROWS, DATA_LENGTH` |
| M13 | 列出 stored procedures | 🟢 L2 | `ROUTINES WHERE ROUTINE_TYPE='PROCEDURE'` |
| M14 | 列出 functions | 🟢 L2 | `ROUTINES WHERE ROUTINE_TYPE='FUNCTION'` |
| M15 | 列出 triggers | 🟢 L2 | `TRIGGERS` 表 |
| M16 | 列出 events | ⚪ L3 | `EVENTS` 表 |
| M17 | 列出 partitions | ⚪ L3 | `PARTITIONS` 表 |

## 三、查询执行（13 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| Q01 | 执行 SELECT 返回结果集 | 🔴 L0 | 核心 |
| Q02 | 执行 INSERT/UPDATE/DELETE | 🔴 L0 | 返回 affected_rows |
| Q03 | 执行 DDL（CREATE/ALTER/DROP） | 🔴 L0 | 用户写啥跑啥 |
| Q04 | 显示执行耗时（毫秒）| 🔴 L0 | "Took 23ms" |
| Q05 | 显示受影响行数 | 🔴 L0 | UPDATE 后看几行被改 |
| Q06 | 显示 LAST_INSERT_ID | 🟡 L1 | INSERT 后看新 ID |
| Q07 | 显示 SHOW WARNINGS | 🟡 L1 | DDL 后的警告 |
| Q08 | 多语句分割（按 ; 切分）| 🟡 L1 | 注意字符串/注释里的 ; |
| Q09 | 多语句一次执行 | 🟡 L1 | 全部跑或遇错停 |
| Q10 | 取消运行中查询（KILL QUERY）| 🟡 L1 | 长查询停止按钮 |
| Q11 | 查询超时配置 | 🟡 L1 | 全局或单次 |
| Q12 | 流式结果（cursor 游标）| 🟢 L2 | 大结果集不内存爆炸 |
| Q13 | 服务端 prepared statement | ⚪ L3 | 性能优化 |

## 四、事务管理（7 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| T01 | BEGIN / START TRANSACTION | 🟡 L1 | 显式事务 |
| T02 | COMMIT 按钮 | 🟡 L1 | 提交 |
| T03 | ROLLBACK 按钮 | 🟡 L1 | 回滚 |
| T04 | 当前事务状态显示 | 🟡 L1 | UI 上看是否在事务里 |
| T05 | 长事务警告（>30s 提示）| 🟢 L2 | 防止卡锁 |
| T06 | 隔离级别切换 | 🟢 L2 | READ COMMITTED 等 |
| T07 | SAVEPOINT 支持 | ⚪ L3 | 嵌套事务 |

## 五、类型映射（MySQL → Domain Value，13 类型）

| ID | MySQL 类型 | Domain Value | 优先级 | 备注 |
|----|-----------|--------------|-------|------|
| TY01 | NULL | `Value::Null` | 🔴 L0 | 必须 |
| TY02 | TINYINT(1) / BOOL | `Value::Bool` | 🔴 L0 | 1=true/0=false |
| TY03 | TINYINT/SMALLINT/MEDIUMINT/INT/BIGINT | `Value::Int(i64)` | 🔴 L0 | unsigned 注意溢出 |
| TY04 | DECIMAL / NUMERIC | `Value::Text`（保留精度）| 🔴 L0 | 不能转 f64 损失精度 |
| TY05 | FLOAT / DOUBLE | `Value::Float(f64)` | 🔴 L0 | — |
| TY06 | CHAR / VARCHAR / TEXT | `Value::Text(String)` | 🔴 L0 | utf8mb4 |
| TY07 | BLOB / BINARY | `Value::Bytes` | 🔴 L0 | 显示字节数 + hex 预览 |
| TY08 | DATE / DATETIME / TIMESTAMP | `Value::DateTime` | 🔴 L0 | UTC 转换 |
| TY09 | TIME | `Value::Text` | 🟡 L1 | "HH:MM:SS" |
| TY10 | YEAR | `Value::Int` | 🟡 L1 | 简单整数 |
| TY11 | JSON | `Value::Json` | 🟡 L1 | UI 折叠展示 |
| TY12 | ENUM / SET | `Value::Text` | 🟡 L1 | 字符串值 |
| TY13 | BIT | `Value::Bytes` | 🟢 L2 | 二进制位 |
| TY14 | GEOMETRY / POINT | `Value::Text`（WKT）| ⚪ L3 | 空间数据 |

## 六、错误处理（10 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| E01 | sqlx::Error → DomainError 映射 | 🔴 L0 | 不暴露底层细节 |
| E02 | 网络错误识别（连不上）| 🔴 L0 | "Cannot connect to host..." |
| E03 | 认证错误（密码错）| 🔴 L0 | MySQL error 1045 |
| E04 | SQL 语法错误（1064） | 🔴 L0 | 显示哪里出错 |
| E05 | 唯一键冲突（1062） | 🟡 L1 | "Duplicate entry for key..." |
| E06 | 外键约束（1452） | 🟡 L1 | 友好提示 |
| E07 | 表不存在（1146）| 🟡 L1 | — |
| E08 | 字段不存在（1054）| 🟡 L1 | — |
| E09 | 权限不足（1142）| 🟡 L1 | "用户 X 无权对表 Y 执行 Z" |
| E10 | 错误日志持久化 | 🟢 L2 | 排查 |

## 七、凭证/安全（8 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| S01 | 密码加密存储（aes-gcm）| 🔴 L0 | 不能明文 |
| S02 | macOS 钥匙串保存主密钥 | 🟡 L1 | 二级保护 |
| S03 | SSL 证书路径配置（CA/cert/key）| 🟡 L1 | 双向认证 |
| S04 | SSL 模式选择（DISABLED/PREFERRED/REQUIRED）| 🟡 L1 | mysql 协议参数 |
| S05 | SSH 私钥认证 | 🟢 L2 | 比密码更安全 |
| S06 | 凭证过期/轮换提示 | 🟢 L2 | 优秀做法 |
| S07 | 主密码 / 启动密码 | ⚪ L3 | 整 App 加密 |
| S08 | OS 钥匙串密码 fallback | ⚪ L3 | 多平台 |

## 八、数据修改（结果集 → DML，7 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| D01 | 双击单元格编辑 | 🟡 L1 | 直观操作 |
| D02 | 自动生成 UPDATE 语句 | 🟡 L1 | 基于主键定位 |
| D03 | 添加新行 → INSERT | 🟡 L1 | — |
| D04 | 删除行 → DELETE | 🟡 L1 | 二次确认 |
| D05 | 批量提交（事务封装）| 🟡 L1 | 全部成功或全部回滚 |
| D06 | 无主键表的处理 | 🟢 L2 | 提示风险 / 拒绝 |
| D07 | 复合主键支持 | 🟢 L2 | 多列 WHERE |

## 九、EXPLAIN / 性能（5 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| P01 | EXPLAIN 文本展示 | 🟢 L2 | 直接展示 EXPLAIN 结果表 |
| P02 | EXPLAIN ANALYZE | 🟢 L2 | 实际执行计划 |
| P03 | 查询计划可视化（树/图）| ⚪ L3 | 实现成本高 |
| P04 | 索引使用情况分析 | ⚪ L3 | 高级 |
| P05 | 慢查询识别 | ⚪ L3 | DBA 向 |

## 十、数据导入导出（8 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| IO01 | 导出 CSV | 🟡 L1 | 最常用 |
| IO02 | 导出 JSON | 🟡 L1 | API 数据 |
| IO03 | 复制选中单元格 | 🟡 L1 | Cmd+C |
| IO04 | 导出 INSERT SQL | 🟢 L2 | 数据迁移 |
| IO05 | 导出 Markdown 表格 | 🟢 L2 | 写文档用 |
| IO06 | mysqldump 风格 SQL DUMP | ⚪ L3 | 调用外部工具或自建 |
| IO07 | CSV 导入（自动推断类型）| ⚪ L3 | 实现复杂 |
| IO08 | SQL 文件批量执行 | 🟢 L2 | 跑迁移脚本 |

## 十一、SQL 处理（8 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| SQL01 | 多语句分割（语义化 ; 切分）| 🟡 L1 | 注意字符串/注释 |
| SQL02 | SQL 词法分析 | 🟡 L1 | 关键字/标识符/字符串识别 |
| SQL03 | LIMIT 自动注入（默认 SELECT 限 500 行）| 🟡 L1 | 防误查全表 |
| SQL04 | SQL 格式化（pretty） | 🟢 L2 | Cmd+Alt+L |
| SQL05 | SQL 关键字大小写规范 | 🟢 L2 | 选项里配 |
| SQL06 | 反引号自动加（标识符冲突时）| 🟢 L2 | 避免 keyword 冲突 |
| SQL07 | SQL 验证（语法预检）| ⚪ L3 | 不连数据库就检查 |
| SQL08 | 完整 SQL parser | ⚪ L3 | 用于补全/重构 |

## 十二、MySQL 方言特性（10 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| DI01 | 反引号标识符 \`column\` | 🔴 L0 | 必备 |
| DI02 | 注释（-- 和 # 和 /\* \*/）| 🔴 L0 | 多语句分割时识别 |
| DI03 | utf8mb4 字符集 | 🔴 L0 | 否则 emoji 乱码 |
| DI04 | AUTO_INCREMENT 识别 | 🟡 L1 | 列详情显示 |
| DI05 | 用户变量 @var | 🟡 L1 | SET @x := 1 |
| DI06 | 系统变量 @@var | 🟡 L1 | SHOW VARIABLES |
| DI07 | LIMIT offset, count（旧语法）| 🟡 L1 | 兼容老 SQL |
| DI08 | INSERT IGNORE / ON DUPLICATE KEY UPDATE | 🟢 L2 | 高级语法 |
| DI09 | INSERT ... SELECT | 🟡 L1 | 子查询 |
| DI10 | EXPLAIN FORMAT=JSON | 🟢 L2 | 现代 EXPLAIN |

## 十三、备份恢复（5 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| BK01 | 单表 DUMP（生成 INSERT）| 🟢 L2 | IO04 同 |
| BK02 | 整库 DUMP | ⚪ L3 | 调 mysqldump |
| BK03 | 从 SQL 文件恢复 | 🟢 L2 | SQL08 同 |
| BK04 | 增量备份 | ⚪ L3 | 企业向 |
| BK05 | 二进制日志查看 | ⚪ L3 | DBA 向 |

## 十四、监控/管理（8 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| MN01 | SHOW PROCESSLIST | 🟢 L2 | 查谁在跑 |
| MN02 | SHOW VARIABLES | 🟢 L2 | 看配置 |
| MN03 | SHOW STATUS | ⚪ L3 | 性能计数器 |
| MN04 | KILL <id> | 🟢 L2 | 杀掉某连接 |
| MN05 | 慢查询日志解析 | ⚪ L3 | 文件解析 |
| MN06 | 错误日志实时查看 | ⚪ L3 | DBA 向 |
| MN07 | InnoDB 状态 | ⚪ L3 | SHOW ENGINE INNODB STATUS |
| MN08 | 连接统计 | ⚪ L3 | Threads_connected |

## 十五、用户/权限（5 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| US01 | SHOW GRANTS（自己的权限）| 🟢 L2 | 排查权限问题 |
| US02 | 创建用户 GUI | ⚪ L3 | 命令行更快 |
| US03 | GRANT/REVOKE GUI | ⚪ L3 | 同上 |
| US04 | 修改密码 | ⚪ L3 | ALTER USER |
| US05 | 刷新权限（FLUSH PRIVILEGES）| ⚪ L3 | 一行命令 |

## 十六、复制/集群（4 项）

| ID | 功能 | 优先级 | 说明 |
|----|------|-------|------|
| RP01 | SHOW SLAVE STATUS | ⚪ L3 | DBA 专用 |
| RP02 | GTID 信息 | ⚪ L3 | — |
| RP03 | 主从延迟监控 | ⚪ L3 | — |
| RP04 | 集群拓扑识别（MGR/Galera）| ⚪ L3 | 高级 |

---

## 量化总结

| 优先级 | 数量 | 累计 | 落地节点 |
|--------|------|------|---------|
| 🔴 **L0 必备** | 26 | 26 | v0.1 出厂前必须 |
| 🟡 **L1 应有** | 49 | 75 | v0.1 GA 时基本齐 |
| 🟢 **L2 增强** | 35 | 110 | v0.2-v0.3 |
| ⚪ **L3 可选** | 30 | 140 | 长尾，可不做 |
| **MySQL 整体** | **140 项** | — | — |

## 不同"实际可用"程度的对照

### "凑合能用"（仅 L0 = 26 项）
- 能连
- 能查
- 能看库/表/列
- 能基础类型展示
- 4-6 周可达
- 像 mycli 但有 GUI

### "个人日常用够"（L0+L1 = 75 项）⭐ **v0.1 目标**
- 多连接、SSL、连接池
- 元数据完整（含 view/index/FK）
- 多语句、事务、取消查询
- 类型完整（含 JSON/ENUM）
- 错误友好
- 行内编辑、CSV 导出
- **18-26 周可达**

### "生产开发用够"（L0+L1+L2 = 110 项）⭐ **v0.2-v0.3 目标**
- SSH 隧道、只读模式
- EXPLAIN
- DML 修改完整
- SQL 格式化
- mysqldump 集成
- **+12-20 周（额外）**

### "DBA 级"（全部 L3 = 140 项）⚠️ **不建议做**
- 用户权限管理 GUI
- 主从复制监控
- InnoDB 内部状态
- 慢查询分析
- **DataGrip 都没全做齐**

## 给 ramag 的最终建议

**v0.1 锁定 L0+L1 = 75 项**，分布在 ROADMAP.md 的 Stage 1-7 里。

**v0.1 GA 后再考虑** L2 的 35 项。L3 的 30 项**明确不做**。

## 与 ROADMAP 的映射

| 优先级 | 在 ROADMAP 的位置 |
|--------|------------------|
| 🔴 L0 | Stage 1 - 4 全部 |
| 🟡 L1 | Stage 1 - 7（含基础 UX）|
| 🟢 L2 | Stage 8 - 13（v0.2 / v0.3）|
| ⚪ L3 | 不在 ROADMAP，明确不做 |

## 结论

| 问题 | 答案 |
|------|------|
| MySQL 整体多少功能？ | **140 项**（按精细化拆分）|
| "实际可用" 至少多少？ | **75 项**（L0+L1）|
| ramag v0.1 目标做多少？ | **75 项 = 实际可用** |
| 一个人完整做完 140 项要多久？ | **9-12 个月**（按 v0.1 4-6 个月、+v0.2 v0.3 累加）|
