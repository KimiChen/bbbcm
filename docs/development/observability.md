# A05 健康、观测与状态公告

本手册是 `plan/02` §17/18、`99` §5.6 和 M0-05 [容量口径](../design-notes/2026-09-capacity-baseline.md)的派生操作说明。仅本机开发；完整客户端测量、真实 PG/Redis 和安全/视觉矩阵在 E01/E02/E04。本轮没有生产 exporter、trace provider、管理公告编辑器或自动事故生成器。

## 启动与故障恢复

按[数据库手册](database.md)在隔离本机配置中先执行现有迁移命令，再启动 Web；状态读取不会创建缺失 SQLite 文件或修复表。使用 `config/default.toml` 的 SQLite 体验模式可内嵌 Worker，独立 `bbbcm-worker` 与内嵌宿主互斥。PostgreSQL 使用独立 Worker 及专用凭证，Web 不读取 Worker 的邮件表。所有 14 项业务 Flag 仍关闭。

Web 与 Worker 使用 JSON 结构化日志。可选 Redis 装配、测试与故障语义见[Redis 手册](redis.md)。提交成功后的 Redis 提示失败只影响唤醒速度，数据库提交结果和持久 Outbox 不变；Worker 总会回到数据库轮询。确定回滚、提交未知或幂等结果重放不会发成功提示。

| 入口 | 实际含义 | 故障及公开边界 |
|---|---|---|
| `/health`、`/health/live` | HTTP 进程可以响应；旧 `/health` 保持兼容探活语义 | 不访问依赖，不表示业务开放或主库健康 |
| `/health/ready` | 主库只读事务能够读取 Flag、公告及 Outbox 状态 | 主库失败返回 503；可选 Redis 失败仅 degraded，不把普通公开读统一挡掉 |
| `/status` | 当前可响应性、公开阅读 Flag 状态、已发布 C0 公告 | 无公告与读取失败分离；失败为 503 和“暂时无法读取”；不显示连接信息、拓扑、内部错误或详细指标 |
| `/__diagnostics/metrics` | 当前进程采集值及主库脱敏快照 | 仅 development/test 且真实 TCP 对端为回环 IP；缺连接信息、远端及 staging/production 均 404。不信任转发头，不公开部署诊断入口 |

新入口禁止缓存。`/status` 使用 A00 本地品牌、原布局组件及类名，新增模板仅在冻结检查中逐路径允许，原文件哈希与组件类名仍逐一校验；未重建 CSS 或改写原基准。它不是 A02 的只读/紧急关闭提示页。正式浏览器、邮件客户端和视觉证据留硬化。

## 公告来源和写边界

`0004_status` 新增 `operations.status_announcements`，持久字段为应用生成公告 ID、maintenance/incident、公开标题/正文、published、published_at、starts_at、ends_at 和正版本。时间使用平台固定 UTC 微秒表示；结束时间必须晚于开始时间，标题不可为空且最多 120 个 Unicode 字符，正文最多 4000。正文按纯文本显示并由 Askama 转义，不能注入 HTML。

发布读取条件是 `published = true`、`published_at <= now`、`ends_at > now`，按开始时间和 ID 排序，最多 50 条。已发布但尚未开始的计划维护可提前展示；草稿、未来发布时间和已结束公告不出现。空表得到明确空态，非法枚举、坏时间、查询失败得到故障态。

PG 的 app_rw 只有 operations schema USAGE 和此表 SELECT，worker/report 无公告授权。写入仅限既有 migrator 维护数据边界，需由负责人员在自己的受控本机事务中使用注册渲染器和绑定值，并先审查内容只含 C0。当前没有公告写 API、CLI 子命令或匿名管理通道；未来管理能力需在 D04 接身份授权、审批与审计，不能把迁移凭证放进 Web。测试通过真实临时 SQLite 和合成公告验证读取，不代替 PG 角色验证。

## 日志与 trace

HTTP 中间件为每个请求生成新的 UUIDv7 关联 ID，并返回 `x-request-id`。不接受客户端传入的 request ID、traceparent、Cookie 或查询参数作为关联标签。task-local trace 随当前事务传到 Outbox；后台生产者没有请求时用自身事件 UUID，显式受信上游 trace 必须通过格式校验。Worker 从已提交事件恢复 trace，sink 及其子事件继续继承。

事务重试、已提交/回滚/未知结果和 sink 结果采用受控字段；原始 SQL 错误、连接串、邮件正文、地址、密码、令牌、有效邀请码和 C3 不进日志或响应。指标标签不包含 request/operation/event/actor ID、IP、原始路径或任意事件类型。关联 ID 只用于脱敏日志/trace，不成为指标维度。受控故障分类保留诊断价值，数据库底层文本仅在开发者受控调试中另行获取，不通过接口返回。

## 指标解释

共享 `Measurement` 区分 `Collected`、`Unsupported`、`NotConnected` 与 `Unavailable`：实际计数为零合法，尚无样本的累计序列省略；读取失败不能变成空表或零。每个进程以固定 UUID `process_instance` 标识计数重置区间，它是采样元数据而非指标标签；重启生成新值。比较窗口须在同进程同重置区间差分；多进程数据由后续受控采集系统汇总，Web 不能冒称已汇总独立 Worker。独立 Worker 每 30 秒记录自己的池、分组序列与活动任务观测。

| 观测 | 本轮实际边界 |
|---|---|
| HTTP | 请求计数、受控 backend/route/method/status/cache_state 分组、源站响应头准备完成延迟。分组 series 使用 s，进程汇总的 `*Ms` 使用 ms，直方图分别返回对应桶边界；它不是完整响应耗时、地区客户端 TTFB、LCP 或性能门槛证据 |
| 数据库 | 事务连接获取耗时/失败、连接占用、BEGIN 至结束尝试、重试和结束结果；按后端和用途分组。连接池读取本进程真实 open/idle/in_use/max，独立宿主各报自己的池 |
| Outbox | 同一只读事务分别观测事件和 sink delivery，列 ready/leased/retry/paused/dlq/completed；PG READ COMMITTED 下多次 SELECT 不承诺同一提交快照，计数差异需结合并发解释。停用暂停、不伪报成功；当前调度把未注册 sink 当永久失败送入 DLQ，只有持久状态为 ready 的暂停记录归 paused。任意自定义 sink 统一归 Other，避免无限维度 |
| 延迟 | 最老 pending 的 `now - occurred_at` 是发生时间年龄；空积压为 0。既有表没有数据库 commit 时间，精确 commit-to-sink 明确 NotConnected，不用发生时间伪造；sink 调用耗时另采 |
| Worker | 领取/完成/失败、续租/丢租、轮询与 sink 的有界分组；poll 为相邻 tick 开始的实际间隔，active_jobs 未接 Worker 时为 NotConnected。取消恢复由实际 SQLite 调度验证。计数不是外部副作用 exactly-once 证明 |
| Redis | 真实执行路径的命中/未命中和依赖错误；关闭时无 Redis 采集值，与启用但失败区分。未运行真实服务时不宣称已验证网络和 Lua 原子性 |
| 邮件 | Worker 用自己的数据库权限每 30 秒读取当前 UTC 自然日截至采样时间的提交 cohort；真实/模拟分开记录。来源未接入显式标明；延迟没有反馈时为 None。率/延迟阈值按显式 Option 配置告警，默认不猜阈值 |
| PG 专项 | lock waits、long transactions、deadlocks、autovacuum、dead tuples、WAL/关系字节等在 SQLite 标 Unsupported，PG 标 NotConnected；没有为采集扩展 runtime 权限 |
| 媒体、搜索、外部错误/分析 | 共享合同与可检查本地输出见[适配器手册](adapters.md)；实际业务/provider 来源未接入，不能从本地模拟推断线上质量或吞吐 |

M0-05 的分组 series 保持 s/B/count 语义；进程汇总 `*Ms`、状态快照和邮件 `*_ms` 单独使用 ms，不可混加。直方图桶是累计计数，超过最后边界的样本仍计入总数；完整 p95 需 E02 原始采样，不使用粗桶插值宣称达标。事务 guard 未确认结束就释放标为 cancelled_or_abandoned，取消 COMMIT future 则为 commit_unknown，二者都不冒称确定回滚。连接占用测到 guard 释放，驱动后续异步清理成本留 E02 单独测量。

## 复现与限制

执行 `./scripts/dev-check.sh` 完成开发期最小集；`cargo test -p platform-db status_repo --locked` 验证真实公告/Flag/Outbox 读取，`cargo test -p bbbcm-cli --test a05_trace --locked` 验证任务关联贯穿实际事务、持久事件和 sink；Worker 集成测试覆盖禁用、丢提示、持续失败后的轮询与取消。日志测试注入合成秘密哨兵，检查 JSON、错误响应和标签没有泄露。

数据库关闭或不可达时 `/status` 与 readiness 失败，liveness 继续可响应；恢复数据库后读取自行恢复。Redis 恢复后重建可丢失状态，数据库租约与去重不变。不对生产做清理、网络故障注入或角色变更，不把测试替身当真实 Redis/provider，不授予 Verified 或任何阶段门禁。
