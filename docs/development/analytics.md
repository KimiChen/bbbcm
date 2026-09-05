# 本机分析流水线

本手册是 `plan/01`、`02`、`06` 与 `99` 的派生操作说明，范围为 WP-MVP-A06 平台切片。身份、内容、可信认可、治理、真正埋点与 `/manage` 看板尚未接线；合成事件测试不能证明 DATA-001..006 产品功能已完成。统计时区为 UTC（+0，DEC-048），自然周/月与有效主题/回复按事实源批准的口径接入，不猜测未批准阈值。

## 事件字典 v1

共享字典为 `platform::analytics::EVENT_DICTIONARY`，字典版本 `1`，Outbox 外壳 `analytics.recorded` v1，投影版本 `events.v1`，sink 名称 `analytics.project`。每个事件由所属领域事务生成并持久化：

| 事件 | 受信生产者 | 源事实与后续接线 |
|---|---|---|
| `registered` | identity | 已提交注册；不含邮箱或邀请码 |
| `email_verified` | identity | 邮箱验证状态提交；不含验证令牌 |
| `invitation_used` | identity | 邀请使用事实；不含邀请码 |
| `read` | interaction | 服务端允许的阅读事实；未来漏斗取每个伪匿名主体的首次阅读 |
| `topic_published` | content | 审核、可见性与内容事实提交；有效性由领域按已批准标准判定 |
| `reply_published` | content | 回复事实提交；未来漏斗与首次有效回复按领域引用与有效性判断 |
| `valid_contribution` | trust | 领域按 `01` §1.3 的当前阶段口径判断的有效贡献；平台不代替它判断字数、认可或处罚 |

`occurred_at` 为业务事实发生时间，构造时规范化到 UTC 微秒；`received_at` 为受信服务将事件追加到事务的时间，不能早于发生时间。消费者记录 `processed_at`。UTC 自然日桶使用发生时间，时间区间为左闭右开；自然日/月合同覆盖闰月与跨年，不随访客展示时区改变。

去重键为应用生成且随领域幂等结果稳定复用的 UUIDv7 `event_id`；同一事件重复执行不得换 ID。领域首次事实及主题/回复源唯一性仍需由领域事务保证。投影消费唯一键为 `(projection, event_id)`。伪匿名主体必须使用独立分析 UUIDv7，不能用账号 ID、Cookie、会话、邮件地址或现有凭据；映射由未来领域服务掌管。字段只允许枚举、数字目录引用、事件时间和伪匿名引用，禁止原始路径、来源 URL、User-Agent、标题、正文、邀请码或任意文本。

管理员、测试账号、机器人、活动参与的排除标记来自当前事务读取的身份、权限和内容事实。`TrustedExclusions` 与 `AnalyticsFact` 不提供 HTTP 反序列化入口；`from_authority` 是受信服务边界，不能把客户端自报值直接传入。存储 JSON 仅在消费者读取数据库已提交来源后解析。四类标记任一为真时仍登记消费 receipt 与水位，但不累计普通分析投影；活动专项分析属于后续能力。

## 事务、读取与恢复

领域服务调用 `analytics_repo::enqueue`，传入当前事务借用连接、受信事实和接收时间。该函数把 Outbox、sink delivery 与最小事实一起追加，提交仍由领域 core 控制。Outbox 只保存事件引用外壳，敏感信息不会转移进安全 payload。服务出错必须回滚整个事务。

应用把 `DbAnalyticsStore` 注入 `AnalyticsSink`，再注册到现有 Worker；独立与内嵌 Worker 复用同一 sink。存储服务拥有独立事务，按 event ID 联查已持久 Outbox 和 `analytics.project` delivery；没有来源、外壳不符或损坏数据均失败。关闭适配返回明确 `Disabled`，存储不可用返回脱敏错误，绝不返回伪造的已处理结果。

每次消费先锁 `watermarks` 中 `events.v1` 根行，再插 receipt、按稳定排序更新聚合，最后更新带版本条件的水位；所有变化同事务提交。PostgreSQL 使用平台集中行锁，SQLite 使用现有 `BEGIN IMMEDIATE`。失败、取消或提交结果未知时，Worker 重试同一事件；receipt 只在确定提交后存在，不会重复累计。

`snapshot` 返回脱敏的 UTC 日桶计数和水位，`window_summary` 返回指定 UTC 时间区间内已消费事实的事件数及去重主体数量。后者需要 worker 权限，返回值没有主体标识；它是计算原语，不能把任意时间区间的 unique actors 自动标注为 DAU、WAU、MAU 或果农指数。空水位的事件时间和延迟为 `None`，不伪装为零延迟。

水位包括消费数、排除数、最大事件/接收时间、最后处理时间、最大处理延迟和版本。最大处理延迟为 `processed_at - occurred_at`；接收差值区分到达延迟，接收至处理差值可由受控诊断读取。最大时间只是观测值，不是跳过旧事件的游标。跨日迟到事件仍回填其实际 UTC 日期。没有产品延迟 SLA 默认值；积压、关闭或失败时水位会停止推进，需要使用已有 Outbox 指标共同解释。

显式调用 `DbAnalyticsStore::rebuild(at)` 可重建本投影；没有未经鉴权的 HTTP 或 CLI 写入口。调用前由受信应用授权、确认处理时间不早于接收时间。重建在根行锁内对已提交来源作一次一致查询，清除本投影的 receipt 与聚合后复用相同 projector，最后原子提交。并行消费者等待根锁；查询后提交的新事实仍有 pending Outbox，随后消费。中途失败回滚完整旧投影。不会删除源事件、源事实、Outbox delivery 或其他投影，也不批准任何保留期或自动清理。

当前重建一次加载来源快照，适合本机合成数据与开发验证；百万数据重建的内存、耗时、分页或代际切换方案属于 E02 硬化，尚无性能达标证据。隐私删除的分析去标识与墓碑传播由后续 privacy 领域接入，当前不得对真实个人数据启动收集。

## 数据与权限

`0005_analytics` 是新增 PG/SQLite 同编号迁移，旧迁移不变。所有对象通过 SchemaRenderer 渲染。

| 表 | 权限 |
|---|---|
| `analytics.event_facts` | app 仅 INSERT，worker 仅 SELECT；report 无权读取伪匿名事实 |
| `analytics.projection_receipts` | worker SELECT/INSERT/UPDATE/DELETE；无 app/report 授权 |
| `analytics.aggregates` | worker 管理可重建投影；report 仅 SELECT 投影、UTC 日、事件类型、维度和值及计数列 |
| `analytics.watermarks` | worker 读取与版本更新/重建；无 app/report 授权 |

SQLite 不伪造数据库角色；受信代码与本机环境保持边界。没有身份、内容、管理权限或审批表，没有 Flag 写入口；14 项业务 Flag 继续关闭。

## 开发验证

执行 `cargo test -p platform-db analytics_repo --locked` 和完整 `./scripts/dev-check.sh`。专用测试使用真实临时 SQLite、合成事件与默认/非默认前缀，验证已提交来源、回滚来源拒绝、伪造外壳拒绝、重复/重启去重、四类受信排除、UTC 跨日迟到、精确延迟、水位空态、聚合、主体去重、消费与重建回滚及 sink 关闭状态。

真实 PostgreSQL/Redis、多实例并发、安全全矩阵、真实用户埋点、生产 provider、百万重建和可视看板未由这些测试证明，依赖后续领域和 E01/E02/E04。
