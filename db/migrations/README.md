# 迁移集合

- `postgres/`、`sqlite/`、`mysql8/` 三套有序 SQL 迁移（N 轨），同一编号代表同一批结构变更（如各集合都有 `0007_invitations.sql`）；`mysql8/` 暂为占位，规范先行，真实迁移落地前不参与对齐校验。
- 文件命名：`NNNN_描述.sql`，从 `0001_init.sql` 起。
- 执行入口：`bbbcm-cli db migrate --backend <postgres|sqlite>`（`mysql8` 为预留值，拒绝执行）。
- 对齐校验：`platform_db::migrations::verify_alignment` 做多集合编号对齐校验；仅空的 `mysql8/` 占位轨可在真实迁移落地前豁免，PostgreSQL 与 SQLite 轨必须始终一一对齐。
- SQL 编写遵守 `plan/02-technical-architecture.md` 第 6.1 节的 PostgreSQL 16/SQLite 3.25+/MySQL 8.0 可移植子集：允许 CTE（`WITH`）与窗口函数；禁 `RETURNING`、`ON CONFLICT ... DO UPDATE`、部分索引，以及该节“语义差异禁用清单”和全局禁用清单中的语法。

当前交付三张必要初始表：迁移元数据由 runner 在带锁的初始化事务建立，`0001_init.sql` 创建另两张骨架表及 PostgreSQL 15 个逻辑 schema/显式 ACL。状态查询和 development/test 种子分别为 `db status`、`db seed`，角色初始化 SQL 输出为 `db bootstrap-sql`。操作、角色配置、错误恢复和清理见 [数据库手册](../../docs/development/database.md)。

物理标识符使用受信模板，完整源字节 SHA-256 与已应用历史绑定；只通过新增编号前向修复。SQLite 重建迁移首行 `-- rebuild`，事务与外键开关由 runner 控制，不允许文件自己提交或切换连接状态。
