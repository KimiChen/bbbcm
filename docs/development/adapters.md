# A06 本地适配器与邮件抑制

本说明是 `plan/01` INTEG-001/005/006、`02` §11/14/17/18 与 `99` §5.6 的派生操作说明，不是独立计划入口。供应商、发送域、凭证、生产限额和保留政策仍待批准。本轮没有 SMTP、真实对象存储、真实恶意检测或生产分析连接。

## 关闭与装配

`platform::integrations::AdapterRuntime::assemble` 从 `AdapterConfig` 装配对象存储、图片处理、扫描、错误追踪、基础分析和邮件合同。默认 `mode = "disabled"`，所有边界返回 `AdapterError::Disabled`，不访问文件或网络。显式 `mode = "local_test"` 只允许 development/test，且必须给出本机私有的 `local_root`；staging/production 拒绝本地模式。使用已忽略的 `data/adapters/` 或临时目录，不指向静态服务目录。构造时拒绝父目录跳转、符号链接及 static/templates/dist/public 路径。macOS 临时目录先使用实际规范路径，避免 `/var` 符号链接。

```toml
# 仅放在被忽略的 config/local.toml 或隔离测试配置中。
[adapters]
mode = "local_test"
local_root = "data/adapters"
```

六个合同都是异步 `BoxFuture<Result<...>>`，应用通过依赖注入替换实现。`Disabled` 表示未开启，`Unavailable` 表示已选择的适配器当前失败，`InvalidInput/Conflict/NotFound` 表示受控输入或幂等错误；错误不附加邮箱、正文、文件路径或底层 provider 文本。Worker 将 Disabled 映射 ExternalDisabled 保留待处理，Unavailable 进入既有重试，永久错误停止自动重试。

`EffectsSink` 消费 `integrations.requested` v1，`MailSink` 消费 `mail.requested` v1；Outbox 只保存 `resource_id`，对应受信 `EffectSource` 或 `MailMessageSource` 回读原始任务。生产来源尚未接入时 `UnwiredEffectSource/UnwiredMailSource` 返回 Disabled。本地测试显式注入合成来源。身份、内容和附件生产者必须在自身业务事务中保存必要来源与 Outbox，然后才由既有 Worker 执行副作用；这里没有新上传、身份、公开反馈或管理写入口。

## 可检查的处理结果

本机目录下的 `quarantine/processed/scan/errors/analytics/mail` 均不公开服务。对象键由受信 UUIDv7 与隔离区域生成，不接受邮箱、用户名、原文件名或路径片段。对象按指定字节预算读写；新文件以临时文件、同步、原子不覆盖链接发布。同键同字节重放成功，不同字节返回 Conflict；不会覆盖已有输出。Unix 目录为 0700，新文件为 0600，拒绝符号链接目标。内存处理和阻塞文件操作在 Tokio blocking pool 执行；取消后的重试依靠同一幂等键核对结果。

- 对象存储实际写入 `.bin`，返回 SHA-256、字节数和 `local_only = true`。`processed` 仍是私有区，名称不意味着扫描通过或附件可发布。
- 图片使用 `image` 0.25.10，仅开启 PNG/JPEG。实际文件头必须匹配声明 MIME；先读尺寸并检查显式像素预算，再施加解码分配预算。按显式目标边界缩小，生成新像素缓冲并重编码 PNG，丢弃原 EXIF、ICC 和文本元数据。失败不生成可用输出。预算来自受信任务，不允许后续客户端自报。完整媒体状态机、附件鉴权和上传流程仍待后续工作包。
- 扫描只识别 `BBBCM-SIMULATED-MALWARE` 合成测试标记，并生成包含内容摘要、字节数、`simulated = true` 的 JSON。返回 SimulatedClean/SimulatedRejected，不能作为真实安全证明。模拟拒绝在 EffectsSink 中返回永久失败。未批准真实扫描器前，业务附件仍关闭。
- 错误追踪 JSON 只含固定依赖类型与固定错误类别；基础分析 JSON 只含固定聚合类型与计数。它们不接受任意日志、URL、用户标识或正文，不代表真实分析产品看板。
- 邮件生成 `.eml`，保留项目品牌、纯文本/HTML 两种正文与内嵌 CID 品牌资产。复用 A00 `email-header.svg`，不访问参考站或邮件外部资源。局部正文做 HTML 转义；主题和地址拒绝换行注入。输出明确标注本地测试，状态仅为 LocalRecorded，绝不宣称已发送或送达。真实邮件客户端兼容与视觉验收留 E04。

事务/安全邮件含 `Auto-Submitted: auto-generated`；营销邮件必须提供受信 HTTPS 退订链接，含 `List-Unsubscribe`，正文有营销退订文案。安全类与普通事务邮件不接受营销退订链接。格式依据 [RFC 3834](https://www.rfc-editor.org/rfc/rfc3834) 与 [RFC 2369](https://www.rfc-editor.org/rfc/rfc2369)；[RFC 8058](https://www.rfc-editor.org/rfc/rfc8058) 的一键退订还要求有效 DKIM 签名，因此当前本地输出不声明 List-Unsubscribe-Post，也不虚构已存在的线上退订端点。SPF/DKIM/DMARC、真实发送和真实反馈鉴权仍待 DEC-011。

## 持久邮件状态与反馈

0006 PG/SQLite 同编号迁移新增四张必要平台表：

| 表 | 最小内容与约束 |
|---|---|
| mail_dispatches | message_id 主键，event_id 唯一且关联 Outbox；受信 recipient_ref、分类、请求摘要、状态、simulated、UTC 时间与版本；不存邮箱或正文 |
| mail_suppressions | recipient_ref 主键；营销退订、投诉、退信标记、硬退信计数与版本 |
| mail_feedback | provider/feedback_id 联合主键，关联 dispatch；固定类别、语义摘要、发生/接收 UTC 时间与 simulated |
| mail_alerts | UUIDv7 主键，provider/feedback_id 唯一；固定告警理由和时间，无原始正文 |

邮件时间按统一 UTC 微秒编码，源消息与反馈拒绝更高精度。反馈的 provider/feedback_id 不可用不同语义重复使用；重新接收时间不改变同一反馈身份。受信来源必须在真实接入时先验证 provider 签名/凭证及 message 绑定，再交给 FeedbackStore；本轮没有公开回调解析器。收件人由原 dispatch 回读，反馈不能自报任意收件人来操作抑制。

固定锁顺序为收件人抑制行 → 消息 dispatch → 反馈身份 → 告警 → Outbox 追加。反馈记录、抑制版本与适用告警/Outbox 同事务提交。反馈来源仅在确定提交后 acknowledge；存储失败保留待处理反馈，提交后回执失败时再次处理会命中持久去重。未知提交不报成功，按同一身份重试核对。Repository 只借用当前事务，DbMailStore 是应用注入的事务存储适配。

规则为：营销退订只抑制 Marketing；投诉和退信抑制该收件人的所有邮件，安全邮件也不能绕过硬退信。硬退信属于永久拒绝，立即停止后续提交并生成告警，无需猜测“连续几次”阈值。不同硬退信反馈会保持停止状态并增加计数；重复同反馈不重复计数/告警。迟到 Delivered 不解除退订、投诉或退信，暂不提供自动解除或自动清理。

抑制发生后，尚在 queued/failed 的消息重试会持久更新为 suppressed。若消息此前已 Accepted/LocalRecorded，而 Worker 回执丢失导致重放，则保留原提交结果并去重成功，不再调用 provider；后来出现的抑制仍阻止同收件人的新消息。

每个硬退信或投诉类别告警与 `mail.alert` Outbox 同事务写入，MailAlertSink 回读固定 DiagnosticCode，再经 ErrorTracker 输出本地可检查 JSON。告警失败由 Worker 重试；告警载荷不携带邮箱、recipient_ref 或正文。以后接真实告警接收方也须通过同一边界。

PG worker_rw 对 dispatch/suppression 只有 SELECT/INSERT/UPDATE，对 feedback/alerts 只有 SELECT/INSERT；app_rw 无邮件表权限。report_ro 只读分类、状态、simulated 与时间列，不读取收件人、消息 ID、反馈 ID 或摘要。无新序列，所有对象受 SchemaRenderer 注册。SQLite 测试不冒充 PG ACL 验证。

## 指标和配置告警

`DbMailStore::metric_snapshot(start, end, simulated)` 返回显式 UTC 半开区间 `[start, end)` 的提交 cohort；送达、延迟、退信和投诉按消息去重，避免 provider 多个反馈 ID 把分子放大。模拟数据独立查询，不算真实投递。无送达反馈时 max_delay_ms 为 None，不填零；数据库不可用返回错误。Accepted 只表示 provider 接收，Delivered 只来自受信反馈。

`MailAlertConfig` 的 min_samples、delivery_delay_ms、bounce_rate、complaint_rate 均为 Option，默认未配置。启用率告警必须显式配置样本下限；使用调用方传入窗口计算，不猜测生产阈值或时间窗口。`evaluate` 返回 Delay/BounceRate/ComplaintRate 固定告警类别。硬退信立即停止和告警是产品类别规则，不受这些可选阈值控制。

## 开发验证与剩余边界

可重复执行：

```bash
cargo test -p platform --lib integrations --locked
cargo test -p platform -p platform-db --lib mail --locked
cargo test -p bbbcm-worker --test platform_services --locked
cargo test -p platform-db --test pool_observations --locked
./scripts/dev-check.sh
```

测试实际建立临时 SQLite、应用迁移、提交 Outbox 引用，并读取本机对象、MIME、扫描与错误 JSON；成功后删除临时处理产物。覆盖同键不同载荷、重启读回、关闭/故障区别、符号链接与预算、PNG/JPEG 解码/净化、营销退订不屏蔽安全类、反馈去重与失败保留、硬退信停止重试及告警事务、投诉/软退信、回滚、模拟分组和时间边界。完整现有 Worker 调度接线由应用测试验证。

连接池由显式 App/Worker/Migrator 构造用途注册，SQLite 内嵌 Worker 也标记 Worker。指标按后端和用途合并，不公开路径、凭证或 pool ID；pools 是至少仍持一个平台包装引用的已构造池数，clone 不重复注册。open/idle/max 读取 sqlx 真实值，in_use 为 open-idle；这些独立瞬时读数遇到并发变化时把 idle 限制到 open。关闭后包装对象尚存时返回 Collected 的零连接，最后包装对象释放后返回 NotConnected，注册表只持 Weak，不延长池生命周期。独立 Worker 每 30 秒输出进程实例元数据、池快照、分组指标和当前任务数；进程实例不是指标标签，用于识别进程重启后的计数新段。临时 SQLite 双用途测试覆盖占用、归还、克隆、同用途合并、关闭和缺失状态。

应用唤醒测试先让真实 Scheduler 完成空轮询，再提交合成 Outbox；唤醒持续不可用及通知全部丢失两种情况下，数据库轮询均恢复消费，并正常响应取消。关闭的外部适配器在本地测试总门开启时仍保持 ready、attempts=0，不进入 DLQ。

需要人工检查 Worker 的实际输出时运行：

```bash
BBBCM_KEEP_ADAPTER_OUTPUTS=1 cargo test -p bbbcm-worker --test platform_services --locked -- --nocapture
```

测试会打印 `data/adapter-probe/<UUID>`；其中 `spool/mail/*.eml` 和 `spool/quarantine/*.bin` 是同一次已提交 Outbox 通过真实 Scheduler 产生的合成邮件与对象，SQLite 保留投递状态。整个 `data/` 已 Git 忽略。该环境变量只由这个集成测试读取；不新增绕过 Worker 的 CLI 或业务写入口。人工复核后可删除该次纯本地合成目录，测试选项不建立生产保留或自动清理策略。

本轮只有开发期验证。真实 PostgreSQL ACL/网络故障、多 Worker 并发、真实 provider/DNS、完整安全/媒体预算攻击矩阵及邮件客户端/视觉验证均未执行，留 E01/E02/E04 或 DEC-011 之后。抑制表不是账号偏好产品入口；真实注册、安全通知、内容附件及管理业务接线仍留后续工作包。保留期与生产告警阈值没有获得批准，不执行自动清理。
