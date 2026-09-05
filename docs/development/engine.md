# 本机事务、Outbox、审计与 Flag 查询

本手册是 [02 架构](../../plan/02-technical-architecture.md) 和 [M0-04 派生设计](../design-notes/2026-09-database-foundation.md) 的操作说明；范围、状态和最新证据只登记在 [99 §5.5](../../plan/99-traceability.md)。仅本机开发，不创建真实账号、生产授权或外部副作用。A05/Redis、A06/provider、身份、内容、管理审批与 Flag 写控制不在本轮。

## 1. 迁移和只读查询

沿用 [数据库手册](database.md) 的隔离 SQLite 配置和现有分层加载器。在仓库根执行：

```bash
cargo run --locked -p bbbcm-cli -- db migrate --backend sqlite
cargo run --locked -p bbbcm-cli -- db status
cargo run --locked -p bbbcm-cli -- db seed
cargo run --locked -p bbbcm-cli -- flag list
```

命令读取 `BBBCM_CONFIG`、本机覆盖及已批准注入层；不要把示例用于非本地库。`seed` 仍只允许 development/test 的四版块与规则占位，不生成账号。`0001` 保持原样，`0002_engine` 增加幂等、Outbox、sink投递和两类追加审计，`0003_flags` 初始化14个注册项的关闭状态与版本1。PG/SQLite编号一致，MySQL只占位。重复迁移校验历史并跳过，不修改checksum或自动接管不兼容对象。

`flag list` 实际读取数据库，分列显示注册默认值、当前值、版本和来源。单条SELECT提供一致快照；缺项显示“缺项安全关闭”且版本为“-”。未知项、非法布尔、非法版本或损坏类型均失败；连接/查询失败返回不可用且非零退出，不打印伪造快照。缺少SQLite文件时不创建数据库。读取接口 `platform::flags::FlagReader` 由 `platform_db::flags_repo::DbFlagReader` 实现，没有写方法。

启动和读取不自动修复、补项、重置或写回；数据库值也不由环境变量覆盖。现阶段只交付查询，未将“数据库存有true”当作业务路由启用批准。测试中的直接写只发生在隔离临时库，不能成为第三个生产写入口。`flag set` 仍明确拒绝，`/manage/config`、共享受控写命令、审计审批、只读/紧急关闭运行时及提示页留给A02/D04后续交付。A02整包保持In progress。

## 2. 应用事务调用

共享合同在 `platform::transaction`，存储适配在 `platform_db::{transaction,command,idempotency_repo}`。应用服务先完成传输校验与授权上下文准备，调用 `command::execute_command`，传入 `CommandIdentity`、完整 `LockPlan` 和只操作当前数据库事务的异步闭包。`WriteCommand<T>` 定义后续领域命令的request ID、实体版本和actor权限版本字段；身份/管理模块须在闭包中重读可变权限事实，不能把调用者传来的权限版本当成授权。

闭包获得 `TransactionConnection` 的短借用，可调用己方Repository及其他领域受限core写服务，不取得连接池或提交能力。幂等命中不执行业务闭包，因此当前身份和对既存结果的访问授权必须在调用模板前完成，不能只放在闭包里。所有值使用 `DbValue` bind，物理表名必须经 `SchemaRenderer`。借用接口拒绝事务控制、多语句与DDL；保存点、BEGIN/提交/回滚留在平台内部。正常模式由模板依次完成幂等预留、排序锁集合、领域写、幂等结果和提交；业务内的Outbox及适用审计都使用同一借用连接。

锁序继承M0-04 §7：幂等 → IAM → identity → trust → community → media/interaction/moderation/support/privacy/operations/campaign/commerce/analytics → 追加记录与结果；同类按受信表序及数字ID排序。`LockPlan` 排序去重，取得业务锁后不能再预留幂等命令，进入追加阶段后不能倒回取领域锁。发现新早序目标时回滚重建计划。当前只有已登记表可参与，未来领域随迁移扩展，不预建业务表。

- PG使用READ COMMITTED、500ms lock_timeout；行锁与SKIP LOCKED只在 `platform::db` 方言入口产生。
- SQLite使用WAL、BEGIN IMMEDIATE、5000ms busy_timeout；不模拟PG行锁。
- `execute_versioned` 必须接带 `WHERE id=? AND version=?` 的条件更新，恰好一行成功，零行版本冲突；调用方必须提供完整版本条件，不能无条件覆盖。
- `insert_or_update` 用INSERT优先保存点；唯一冲突先回滚保存点再UPDATE。唯一冲突是业务冲突，不消耗事务重试预算。`last_insert_id` 在同一事务同一连接回读刚插入的单行，批量改用预定唯一键回查。
- PG 40001/40P01、SQLite BUSY/BUSY_SNAPSHOT只有具备有效幂等键且确认回滚才重试，初次执行之外最多3次。锁超时、唯一冲突或未知提交不进入该预算。模板闭包不得发送外部请求，因为它可能重新执行。

只有确定commit/rollback完成才允许连接归池；BEGIN或结束future取消、结束失败/未知、遗漏显式结束均由RAII start_rollback并close_on_drop丢弃原连接。关闭连接不证明未知提交已回滚。不要捕获并忽略Repository错误；让它传播到模板完整回滚。错误文本仅包含固定分类，不输出驱动原文、SQL参数或连接串。

## 3. 幂等与未知提交结果

唯一键为 `(scope, actor_id, idempotency_key)`；请求先按领域合同规范化，再对规范字节哈希。operation_id用 `platform::transaction::new_operation_id()` 在提交前生成UUIDv7，同事务保存。同key同载荷返回原operation/结果，不运行闭包；不同载荷报冲突。结果只允许 `SafePayload` 的最小引用与固定outcome，不保存Secret/C3、响应头或任意正文。

普通响应从创建起30天后只返回 `ResponseExpired`，不自动重新执行业务。本轮不实施物理清理；到期清理能力接入时只可清普通响应，不能删除邀请消费、处罚、授权、账本或资格等业务唯一事实。审计/Outbox/DLQ/投递去重的保留期仍Pending，不把没有清理器当作永久保留批准。

调用方必须处理 `CommitOutcome::Committed`、确定失败回滚，以及 `Unknown { operation_id }`。提交连接中断不能伪装成功；未知后不盲目重放。未来HTTP层按01 §7.3映射202及operation ID，本轮没有HTTP操作查询入口。

受控查询合同为 `OperationQuery { actor_id, scope, idempotency_key, operation_id }` → `idempotency_repo::query_result`。只能由已验证原actor的应用服务调用；operation ID本身不是访问凭据，四项任何不匹配都不能取到他人结果。`Completed`才对应已观察到的确定结果；`NotObserved`仅表示本次未查到，不能据此判定原提交已回滚，也不能触发自动重放。`Pending`、`ResponseExpired`、存储不可用均单独处理。无身份系统时只在合成测试上下文中调用。

本轮“丢失提交确认”和“回滚后丢失确认”的故障测试是可控模拟；真实PG网络断连、序列化及死锁完整矩阵由E01补齐。

## 4. Outbox 与两个 Worker 宿主

`platform_db::outbox_repo::enqueue` 接受当前事务、受信renderer、`OutboxEvent`与目标sink列表。事件包含UUIDv7、聚合引用、事件类型/版本、UTC微秒时间、追踪ID和 `SafePayload`。载荷仅接受登记的opaque引用和固定outcome；任意字符串、秘密和原始C3不进入事件。`WebhookEventV1` 是INTEG-004的版本化线格式预留，没有第三方订阅创建或网络投递。

`sink_deliveries` 以 `(sink,event_id)` 唯一，记录每个目标sink完成状态。事件状态为ready/leased/delivered/dlq，重试时间、尝试数、租约和重放次数持久化。PG用集中方言SKIP LOCKED批量取锁；SQLite先取ID批、条件更新租约并核对影响行数、再按UUIDv7 lease_batch_id回读。lease_owner仅供诊断，不作为批次回读条件。旧owner/旧batch或已过期租约不能续租、确认成功或改失败状态。

`platform_outbox::Scheduler` 是独立worker和web内嵌jobs唯一调度实现，`OutboxStore`由应用注入。当前使用轮询，未实现Redis唤醒。sink注册表拒绝重名，未知sink进DLQ；已持久化确认成功的sink不因其他sink失败再次执行。长任务会续租当前及尚未执行的批次项；取消停止领取、释放仍持有的未完成租约。进程崩溃留下的租约到期可回收，重启沿数据库状态继续。

开发调度默认值在 `Settings` 中明确：轮询1s、租约30s、续租10s、批量16、失败预算5次、指数退避1s至300s。它们是当前引擎的可注入调度参数，不是生产配额、保留期、容量结论或性能SLA；生产连接池等仍不设猜测默认。

```bash
# 使用已迁移的本机SQLite，独立worker配置必须设置worker.embedded=false
cargo run --locked -p bbbcm-worker
# 内嵌宿主使用worker.embedded=true，由同一bbbcm-web配置启动
cargo run --locked -p bbbcm-web
```

SQLite宿主对规范化数据库路径的 `.worker.lock` 取得OS独占锁，配置校验和文件锁共同保证两种宿主互斥；先取得锁并准备存储，再启动服务。不要删除活动锁文件；OS锁在退出或崩溃后释放，文件存在本身不表示持有，残留空文件不需“恢复删除”。仅使用适合本机文件锁语义的本地文件系统；跨平台和网络文件系统验证留硬化。配置、库/WAL/SHM与锁文件继续忽略。

真实宿主只注册本地probe和关闭的audit.replica，注入 `ClosedExternalEffects`；没有真实邮件、Webhook或provider请求。关闭不把投递标记为成功，不耗尽普通失败预算；恢复后仍需经批准配置和对应服务接线。数据库去重无法消除“外部调用已成功、成功记录提交前进程崩溃”的窗口，外部provider仍须使用event ID幂等或对账合同，不能声称天然exactly-once。

## 5. DLQ、受控重放和恢复

可重试sink错误指数退避，超过预算进入DLQ；永久错误或未注册sink直接DLQ。错误码为固定枚举，不持久化provider原始错误。DLQ重放通过 `DbOutboxStore::replay` 与注入的 `ReplayPolicy`，没有HTTP或CLI默认allow入口。`ReplayPolicy::authorize`每次执行（含幂等命中），新操作才锁定DLQ事件、更新状态、调用`append_audit`并完成幂等记录；确定回滚时重放状态不变化。`ReplayResult.replayed`标明原结果；`ReplayConflict`与`ReplayExpired`分别拒绝载荷冲突和过期操作。`CommitUnknown`不能据此判断状态未变：查询必须携带原actor、scope=`outbox.replay`、idempotency_key=原operation_id、原operation_id；不得换新ID重试。operation ID、原actor及规范化重放载荷绑定幂等，旧操作不能重复用于后来再次DLQ的事件；已完成sink的去重事实继续保留。

恢复顺序：先用 `db status` 确认完整迁移历史；修复本地连接/配置或注册缺口；重启单一合法宿主；等待未释放租约到期回收。副作用关闭及DLQ不由启动自动修复或重放。需要重放时由后续授权服务调用合同并记录理由/操作ID，不直接UPDATE表来跳过审计。业务事实已在原事务正确提交，不能靠Worker重建权限、积分、资格或内容的权威状态。

## 6. 追加审计与敏感读取

`append_audit` 同事务写入类型化 `AuditEvent` 并加入audit.replica Outbox请求；任何插入失败都传播并使高危命令回滚。副本请求只包含audit_event_id，`DbAuditExport`在受信适配中读最小投影，再由 `ReplicaSink` 交给 `AuditReplica`；真实宿主使用 `DisabledReplica`，本地测试适配器验证追加/去重，不代表独立生产权限域已经获批或建成。

审计保存actor、类型化动作/理由/目标、工单引用、审批关联、前后摘要、operation/trace、统一时间与版本；输入类型当前只登记合成与引擎动作，未来领域按需求扩展。C0–C2前后值仅保存摘要，Secret/C3连摘要都不记录；不接任意正文说明。更正追加新事件的correction_of引用，不UPDATE旧事件。PG显式ACL只授runtime INSERT，worker另获复制所需的列投影SELECT，不授UPDATE/DELETE/TRUNCATE；report角色没有原始审计权限。SQLite没有数据库角色，依赖受控应用边界和本地文件权限，不伪造PG ACL保证。

敏感读取调用 `platform_audit::read_sensitive`，注入 `DbSensitiveSession` 和同事务 `ReadPolicy`：重新授权 → 内部暂存允许投影 → 追加SensitiveAccessEvent → 确定提交后才交出数据。Session阶段接口不会在提交前返回暂存数据；未授权、非法事件、非成功结果、读取失败、审计失败、提交未知都失败关闭并回滚或丢弃。访问审计必须匹配已授权的完整上下文，不能换actor/目标后记录；C3只允许专用验证结果。当前没有真实身份、管理员或未经鉴权的读取路由，合成policy仅位于测试。

## 7. 开发验证与硬化边界

统一运行 `./scripts/dev-check.sh`，包括格式化、Clippy、workspace测试、真实SQLite迁移与既有HTTP回归、diff和暂存复核；不增加独立构建或完整资产构建。相关测试在platform共享合同、platform-db事务/幂等/Flag/Outbox、CLI的应用审计组合以及worker调度/子进程中，实际使用临时SQLite和本地sink。

容量模型、生成方式和A05/E02指标输入见 [M0-05模型](../design-notes/2026-09-capacity-baseline.md)。真实PG双前缀/ACL/网络故障、Redis、多Worker并发、安全全量、跨平台、性能及视觉完整矩阵未在本轮执行，留E01/E02/E04。全部产品验收与正式门禁仍以99记录为准。
