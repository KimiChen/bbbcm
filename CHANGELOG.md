# 变更日志

本文件记录对使用者可见的变更。格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

项目尚无发布版本；开发期的工作包状态与证据见 [`plan/99-traceability.md`](plan/99-traceability.md)。

## [未发布]

### 新增

- AGPL-3.0-only 许可证与静态资产的许可边界（`DEC-023`）。
- `AUTH-014` 首个站点所有者的一次性 CLI 引导与强制 MFA 绑定（`DEC-031`）。
- 开发期最小集脚本入口 `scripts/dev-check.sh` 与冒烟路由清单。
- `Wakeup`、`Lease` 两个平台抽象与进程内实现。
- 跨库时间文本编码规范与 `encode_time`/`decode_time`。
- 备份与恢复规范（`plan/02` 第 18.1 节、`plan/91` 第 7 节）。

### 变更

- 限流由 Redis 承载；`features.redis = false` 时不提供限流（`DEC-024`）。
- SQLite/MySQL 物理表名改为携带逻辑 schema 前缀（`DEC-029`）。
- Feature Flag 有两个受控写入口：`/manage/config` 与 `bbbcm-cli flag set`（`DEC-028`）。
- 根目录设计稿退役为 `docs/design-notes/`，`plan/` 成为唯一计划事实源（`DEC-026`）。
- `plan/01` 的数据模型与事务边界同步到三库可移植子集（`DEC-022`）。

### 修复

- `render_sql` 不再把 SQL 注释中的 `?` 当作占位符，并拒绝双引号标识符与写死的 `$N`。
- `SchemaRenderer` 校验逻辑 schema 与表名，并检查标识符长度上限。
- 进程内限流器与内存缓存改为有界，避免客户端可控 key 耗尽内存。
- 内部错误详情不再进入 HTTP 响应体，只记录到日志。
- `bbbcm-web` 与 `bbbcm-worker` 处理 SIGTERM，内嵌任务循环支持取消。
- `Id<T>` 拒绝负值、零与非规范十进制字符串。
- FTS5 探针在同一条连接上创建与删除，不再残留临时表。
