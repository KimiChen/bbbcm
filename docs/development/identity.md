# 本机身份开发与恢复

本手册是 [计划入口](../../plan/README.md) 下的操作说明，范围与需求状态统一见 [99 §5.7](../../plan/99-traceability.md#57-b01b02b04-身份开发登记2026-09-05dec-050)。产品取值和批准记录以 `plan/01`、`02`、`06` 为准；本页不建立第二份计划，不授予 Verified 或阶段门禁。

## 当前可以运行什么

当前交付可运行隔离数据库中的开发测试。正常启动 Web 并不能体验完整注册、登录和账号设置：

- `apps/web/src/identity_web.rs` 装配 `UnapprovedIdentityPolicy`，用户名、邮箱、简介、预认证期限、新邮箱期限和邀请窗口等待决输入返回未批准；没有用于开启这些策略的配置键或 CLI。
- `IdentityService` 没有装配 `TokenIssuer`，风险规则为 `UnconfiguredRiskPolicy`；Web 的独立风险摘要密钥为 `None`。规则版本 `0` 表示未配置/不可用，不能表示允许、T1 或邀请授权。
- 所有业务 Feature Flag 注册默认关闭。数据库缺项安全关闭；迁移不启用业务 Flag。未批准策略、缺失邮件密钥、未配置风险规则不会因为存在页面代码而被跳过。
- `apps/worker_services.rs` 的正常 Worker 宿主仍注入 `UnwiredMailSource`。即使已有本地 provider，正常 Web/Worker 组合也尚不能生成并消费身份邮件。这一待接线边界不能用测试成功替代。

测试夹具使用实际临时 SQLite、合成政策版本、普通账号及明确标识的本地 provider。夹具内受控启用 Flag、构造合成信任事实仅服务于测试，不是运维操作入口。仓库没有本轮可用的 Flag 写入、授信、管理员创建或首账号提权入口；CLI 中相关占位命令不构成实现或授权。

资料投影包含已有账号数据、简介、加入时间及本地预设头像；信任状态取受信快照。内容列表、贡献统计、领域角色数据和完整集合页未接入；上传及 WebP/AVIF 转码留 C03。首次阅读、回复、贡献和管理看板也不由本轮身份事件接线提供。

## 数据库检查与迁移

先按 [数据库手册](database.md) 核对实际加载的配置及本机数据库路径，保留既有数据。在仓库根目录只读检查：

```bash
cargo run --locked --package bbbcm-cli -- db status
cargo run --locked --package bbbcm-cli -- flag list
```

仅在已确认的本机 SQLite 数据库应用现有迁移：

```bash
cargo run --locked --package bbbcm-cli -- db migrate --backend sqlite
cargo run --locked --package bbbcm-cli -- db status
```

`0007_identity.sql` 在 PostgreSQL/SQLite 两轨配对新增账号、凭证、普通会话、一次性令牌、邀请、政策同意、用户名占用及历史、预认证主体、邮件请求及收件人映射；同时增加信任快照、邀请使用事实、风险事件和政策版本读取表。所有对象经受信注册与 `SchemaRenderer` 渲染。PostgreSQL 同迁移配置最小表/列/序列 ACL；Worker 不获得用户资料、密码或会话读取权限。SQLite 不模拟数据库角色。

迁移没有种子账号、政策正文、用户授权或 Flag 开启状态。既有 `db seed` 仅保留版块及规则占位，不满足三份注册政策。MySQL 仍为空轨占位；本轮不运行真实 PostgreSQL 迁移。修改已应用迁移、改 checksum、清空数据库或手工补授信都不是恢复步骤。迁移提交结果不明时先重跑只读 `db status`，遵循 [基础引擎手册](engine.md)。

## 代码装配合同

这些是可复核的受信构造接口，不是运行时配置绕过方式。待决事项批准并更新事实源后，才可实现对应正式装配；当前示例来源应查看测试文件，不复制夹具 SQL 到日常数据库。

| 合同 | 责任与当前边界 |
|---|---|
| `IdentityPolicy` | 注入用户名/邮箱规范化、反冒充及简介检查、预认证期限、新邮箱期限、邀请窗口及额度；当前正常装配为 `UnapprovedIdentityPolicy`。 |
| `PasswordEngine` / `PasswordScreen` | Argon2id；已批密码长度为 8–256 字符，最小长度可配置。当前泄露筛查为显式 `DisabledPasswordScreen`，不调用外部服务；验证码及泄露密码的最终行为仍受 DEC-011/CR-014 约束。 |
| `platform_db::policy::PolicyReader` | 接受当前事务连接，读取并核对条款、隐私、社区规则三个当前版本 ID。应用注入 `CurrentPolicyReader`，避免 identity 对 moderation 产生反向领域依赖。合成版本仅 development/test 可用；没有发布、审批或法律文本编写接口。 |
| `IdentityService::with_token_issuer` | 显式注入实验性令牌派生密钥及版本；正常装配没有密钥，也没有 Secret 加载入口。可靠重试设计的产品批准仍待完成。 |
| `RiskPolicy` / `RiskSignalKey` | 规则与独立摘要密钥由受信装配提供；版本化结果说明允许、拒绝、挑战或不可用。HTTP 不能声明角色、等级或邀请权；风险摘要使用带用途分隔的 HMAC，网络段摘要待批准前为空。 |
| `with_registration_analytics_exclusion` | 从受信来源确定注册账号的分析排除位，默认排除未配置本地运行。三个身份事件在当前事务读回该事实，不接受表单自报；后续管理员、机器人和活动事实仍需相应领域接线。 |
| `IdentityMailSource::new_local` | 接受数据库、受信渲染器、按版本保留的 issuer 列表、合成发件地址及严格回环 HTTP 基址。测试注入现有 `MailSink`、`DbMailStore` 和 `LocalMailProvider`，经现有 Scheduler 执行。 |

跨领域写只经接受当前事务连接的受限 core 服务。邀请消费、账号与凭证创建、三个政策同意、初始信任事实、幂等响应以及必要 Outbox 共同提交。追加邮件、分析或风险记录前完成需要的业务锁；服务不另开事务提交其他领域事实。

新账号的信任快照为 `pending`、未验证、无邀请权。邮箱确认只更新验证事实；资格待决时仍不升级为 T0/T1，不授予邀请权。已批统计可信用户口径中的“账号年龄不设阈值”不能用于这些授权判断。

## 邮件、令牌与重试

验证令牌有效期 24 小时，找回密码令牌 1 小时；消费须检查目的、账号归属、当前凭证版本、过期时间及单次状态。新邮箱确认期限与重发规则待批准。当前受控实现的新邮箱请求会替换该账号先前未完成的新邮箱令牌；该行为也不能被解释为已经批准了通用验证邮件重发规则。

当前实验设计用外部密钥版本、随机 nonce、令牌引用、账号、目的、目标邮箱规范键、到期时间及凭证版本派生 bearer。数据库保存校验哈希和重建上下文；不保存原始 bearer 或可解密 bearer。Outbox 仅有不可变消息引用，不携带邮箱、正文或原始令牌。此设计是在验证“只存令牌哈希”与可靠异步邮件之间的实现假说，尚未取得正式装配批准。

只有本地投递时才在内存重建正文。邮件给出不含 query token 的回环页面地址，提示用户粘贴令牌。原始令牌会出现在 Git 忽略的私有 `.eml` 测试产物内，不应复制到日志、工单、截图或入库证据。邮件复用 A00 本地品牌资产，provider 输出 `X-BBBcm-Delivery: local-test-only`；`LocalRecorded` 只表示写出本地文件，不表示真实发送或送达。

重试按已有 Outbox `event_id`、消息引用和持久投递记录恢复：

1. Source 按不可变上下文重建同一正文；令牌已经消费、凭证已经更新也不改变旧消息，以便核对既有投递记录。令牌失效由消费事务负责；迟到邮件不能重新获得有效性。
2. `DbMailStore` 检查持久抑制及投递记录。规范化邮箱映射到稳定 `recipient_ref`，同一邮箱不会因展示拼写变化绕过抑制。投诉/硬退信等按 A06 既有规则处理；营销退订不能据此屏蔽安全邮件。
3. provider 暂时不可用时，由既有 Scheduler 保留失败状态并重试；重启重建相同服务后继续同一事件。抑制是持久终止结果，不通过更换 message/event ID、删除抑制行或伪造反馈恢复投递。

只要旧消息仍可能按版本加载或重放，就不能移除对应旧密钥。缺少旧版本密钥会使 source 返回不可用；保留同一上下文、恢复受信密钥后重试。当前没有批准密钥保留期、令牌/消息清理期或清理命令，不能从令牌 1/24 小时有效期推导为安全删除消息或密钥的期限。密钥丢失、永久 source 损坏和重试耗尽应保留失败事实，按 [适配器手册](adapters.md) 与引擎合同处理。

本轮已接密码与邮箱变更安全通知。异常登录通知或风险挑战的触发条件仍待批准；存在 `login_security` 模板并不表示该触发流程已经交付。

## 会话、未知提交与本机恢复

普通会话只保存 token/CSRF 哈希，绝对期限 30 天、空闲期限 14 天；重新认证窗口 5 分钟。资料与设备写入重读当前账号和会话，校验 owner、CSRF 及适用版本。改密码、重置密码或确认新邮箱撤销全部旧会话并使旧凭证版本的令牌失效；设备撤销不暴露其他账号的会话。

业务提交结果未知时返回 HTTP 202 和 `operation_id`，页面保留原操作查询表单。使用同一浏览器中的原证明及表单，通过 `POST /auth/operations` 查询原 actor、scope、幂等键与 operation ID；不要换键重做可能已经完成的业务操作。

- 公开流程使用原预认证主体及 CSRF 证明。普通账号流程使用原 session bearer 的哈希及 CSRF 证明；即使退出或密码变更已经撤销会话，该受限查询仍可在原绝对期限内读取原操作摘要。它不会创建 `SessionIdentity`，也不能授权业务写入。
- `Completed` 只确认已存结果。登录凭据及邀请码只展示一次，不从幂等响应恢复秘密；若凭据未收到，按结果页重新进入相应流程。
- `Pending` / `NotObserved` 不等于已回滚，继续查询原操作。响应过期、原证明丢失或已过期时不能假定失败再执行；当前没有管理员救援入口，应保留脱敏操作引用供后续核查。
- 页面与响应使用 no-store、no-referrer；日志仅记录生成的请求关联、封闭路由类别、状态与耗时，不记录 URI、query、请求体、Cookie、邮箱或令牌。

Redis 关闭时四维限流明确为 `NoProtection`；Redis 已开启但不可用时按 A05 高风险故障合同失败关闭，不回退到内存保护。账号/IP/设备/网络四维参数未批准时不创建生产默认预算；风险摘要密钥与邮件派生密钥相互独立。

清理仅针对已确认的本机合成测试目录。先停止相关 Web/Worker、关闭连接，再处理指定测试数据库及其 WAL/SHM、对应私有 spool；不要删除正在等待重试的来源、receipt 或抑制记录来“修复”业务状态。测试夹具通常自行删除临时目录；若进程中断遗留产物，核对独立临时目录归属后清理。`data/`、`.local/`、本机配置、数据库、邮件和秘密继续忽略，不提供生产清空或自动保留策略。

## 开发验证入口

在仓库根目录执行；测试自行建立临时 SQLite，不依赖正常运行库已有账号或 Flag 值：

```bash
cargo test --locked -p identity --lib
cargo test --locked -p trust -p moderation --lib
cargo test --locked -p platform --lib sql_lint
cargo test --locked -p platform --lib observability::grouped
cargo test --locked -p platform-db --lib migrations
cargo test --locked -p bbbcm-cli --test database_cli
./scripts/dev-check.sh
git diff --check
```

`identity` 的 core/settings/HTTP 测试覆盖注册事务、单次令牌、会话与 owner、合成邀请授权及额度、用户名保留、邮件和分析排除。`scheduler_failure_retry_and_recreated_worker_resume_persistent_identity_mail` 已经使用实际临时 SQLite 和现有 Scheduler，先模拟 provider 失败，再重建 Worker 服务写出本地邮件，核对持久状态及再次重启不重复输出；这不是正常 Worker 宿主已接通身份 source 的证据。持久抑制测试实际拒绝本地输出，并重建存储后复验。完整命令结果与本轮候选证据仍以 99 §5.7 为准。

这些命令不证明真实 PostgreSQL/Redis、生产 IAM、真实供应商、真实发送/反馈、网络提交未知、多 Worker 并发、完整安全/性能或正式视觉矩阵通过。相应验证留统一硬化 E01/E02/E04，不授予 Verified。

## 待批准与后续接线

以下事项集中回到事实源决策流程；尚未确认的值不写成默认参数：

- T0/T1 实际资格、邮箱验证与信任等级的关系、邀请生成资格、是否及如何升级；邀请有效期、每日额度日界线、撤销/过期后的额度释放及按等级调整。
- 四维限流预算与时间窗口、网络前缀、风险挑战触发及验证方式；验证码、泄露密码筛查的供应商、故障行为和完整 DEC-011/CR-014 子句。
- 用户名与邮箱规范化、字符合法性、大小写/别名处理、反冒充保留名及相似名规则、简介限制；已批用户名 30 天冷却和 90 天历史防抢注继续沿用。
- 预认证证明期限、新邮箱确认期限、验证/恢复邮件的重发与过期恢复行为；派生令牌假说、密钥保管/轮换及旧版本恢复边界，以及尚未批准的数据保留期。
- 正式三份政策文本、发布和审批，正常 Web/Worker 的身份 source 及 Secret 装配；不借最小 MOD-010 读取切片提前实现后台发布。
- DEC-049 字符折算及自然周起点仍待确认，不阻塞本轮注册、验证和邀请使用三个事件的共同提交。账号年龄的统计口径不改变身份或邀请权限。

MFA、管理会话、首账号提权、完整后台、内容发布、账号导出/注销与删除传播、积分、真实 provider 和头像上传均不由本手册提供操作入口。包状态及逐需求缺口只在 99 更新。
