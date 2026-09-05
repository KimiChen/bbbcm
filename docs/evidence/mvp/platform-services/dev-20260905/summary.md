# A05 → A06 开发交付证据

状态事实源为 [99 §5.6](../../../../../plan/99-traceability.md)。A05 为开发期 Implemented，A06 与 A02 整包继续 In progress，14 项业务 Flag 保持关闭。只做本机提交，无 push、部署、真实 provider 或真实邮件。

本轮承接现有 Rust/sqlx/Outbox/Worker：Redis 可丢失状态与故障分流，有限观测/trace、真实健康和 C0 公告；持久分析去重/投影/水位/重建；六类关闭/本地适配与邮件反馈抑制/告警。仅新增 0004–0006 双轨迁移，旧迁移未改，无 platform-* 横向依赖。redis/image 为必要新增依赖，现有包未升级。

完整开发最小集通过 182 项 Rust 测试和 55 条 HTTP 冒烟，包含真实临时 SQLite 六迁移、双前缀、原引擎/Flag/Worker/A00/A01 回归及本轮针对性测试；真实 Redis 合同 1 项 ignored。另 10 项容量测试、原 A00 冻结检查通过。最新运行结果与命令日志见 validation.json；逐文件暂存检查另记 staged-review.json，不能用 dev-check 当时暂存为空代替复核。

独立复核与修正见 independent-review.md。失败日志按原运行分别保留：早期缺依赖/类型导入、Askama 数字转义断言、旧迁移编号断言、配置测试使用 parse 而未 validate、Repository 动态 SQL/内联测试路径拼接，以及并行 Pool 合同尚未全部落盘时的暂态编译失败。最终检查通过不抹去这些历史，也不把暂态编译错误当真实系统故障。

local-output-inspection.json 是对真实 Scheduler 生成的合成 MIME 与对象文件的读取摘要：邮件含 text/plain、text/html、内嵌 SVG 品牌三部分，状态 LocalRecorded。原始文件及数据库位于 Git 忽略的 data/adapter-probe；不把原始邮箱、正文或本机路径放进公开证据。

未执行真实 PG/Redis 原子性、角色全矩阵、真实 provider/DNS、安全/多实例/跨平台/性能/视觉完整验证；未接入完整客户端延迟、精确数据库 commit-to-sink、PG 系统统计、真实领域生产者及看板。没有 Verified 或任何阶段/发布门禁声明。自然周起点和 Unicode 字符折算按 DEC-049 等待用户答复，其余批准参数已同步事实源。

决策先行提交为 6fa32f7、bb0b584；实现版本为本证据所在 `新增：交付 A05 平台服务及 A06 分析邮件底座` 提交。操作入口为 docs/development/redis.md、observability.md、analytics.md、adapters.md。安全配置模板和代码正常版本化，AGENTS.md、g.md、本机配置、Secrets、缓存、数据库和处理输出保持忽略。
