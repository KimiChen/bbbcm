# 迁移集合

- `postgres/`、`sqlite/`、`mysql8/` 三套有序 SQL 迁移（N 轨），同一编号代表同一批结构变更（如各集合都有 `0007_invitations.sql`）；`mysql8/` 暂为占位，规范先行，真实迁移落地前不参与对齐校验。
- 文件命名：`NNNN_描述.sql`，从 `0001_init.sql` 起。
- 执行入口：`bbbcm-cli db migrate --backend <postgres|sqlite>`（执行器随对应工作包接入；`mysql8` 为预留值，未实现时拒绝执行并提示）。
- 对齐校验：`platform_db::migrations::verify_alignment` 做多集合编号对齐校验；仅空的 `mysql8/` 占位轨可在真实迁移落地前豁免，PostgreSQL 与 SQLite 轨必须始终一一对齐。
- SQL 编写遵守 `plan/02-technical-architecture.md` 第 6.1 节的 PostgreSQL 16/SQLite 3.25+/MySQL 8.0 可移植子集：允许 CTE（`WITH`）与窗口函数；禁 `RETURNING`、`ON CONFLICT ... DO UPDATE`、部分索引，以及该节“语义差异禁用清单”和全局禁用清单中的语法。
