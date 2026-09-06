# 普通身份流程：本机操作与恢复

范围对应 `99` §5.8、DEC-050/051。正式产品/技术规则以 `plan/01`、`02`、`06` 为准；本页不授予 Verified、阶段门禁或发布批准。

## 当前装配与隔离边界

Web 正常装配 `ApprovedIdentityPolicy`、Argon2id/常见弱密码筛查、`CurrentPolicyReader`、风险规则 v1 和 A05 四维预算。显式注入本地邮件配置与外部 Secret 后，Web、内嵌及独立 Worker 使用相同版本化令牌合同，Worker 注入 `IdentityMailSource`。没有完整身份邮件/Secret配置时，普通身份服务不装配、source保持不可用；其他领域开启PublicRead也不会开启缺配置的身份写入，不生成默认密钥。

所有业务 Flag 注册默认关闭，数据库缺项也关闭；本轮不提供 Flag 写口、配置绕过、管理员/首账号引导或授信命令。只有测试代码在隔离临时数据库内构造受控启用状态、普通邀请账号及三份明确标注的合成政策。正常启动后看到关闭页是预期行为，不应将夹具 SQL 复制到日常数据库来开启业务。政策正式文本、发布及审批仍待后续。

新账号信任快照为 `pending`。邮箱验证在同一事务更新验证事实和邀请资格：active 且已验证即可生成邀请，不自动授予 T0/T1；已有等级保持。邀请默认三天、可选整数1–7天；每日累计创建5个、有效未用最多5个，UTC零点分日。撤销、消费、过期释放未用库存；撤销不退当日累计创建次数。

账号规范、简介上限、用户名30天冷却/90天历史防抢注在服务端执行。个人主页只投影真实账号字段、信任状态和冻结的预设头像；内容列表、贡献统计、其他领域角色与完整集合页未接入，不填合成数字。上传/转码留C03，账号注销/删除传播等不在范围内。

## 数据库与本机配置

按 [数据库手册](database.md) 确认加载配置及实际本机数据库，保留已有数据：

```bash
cargo run --locked --package bbbcm-cli -- db status
cargo run --locked --package bbbcm-cli -- flag list
cargo run --locked --package bbbcm-cli -- db migrate --backend sqlite
```

`0007_identity.sql` 保持原样；新增配对 `0008_identity_challenges.sql` 只登记挑战哈希、随机nonce、预认证主体/动作/请求绑定、密钥版本、期限和单次状态。PG只给app必要读写权限，Worker不读取挑战、用户、凭证或会话；SQLite不模拟数据库角色。MySQL继续占位。迁移不写账号、政策正文、授信或Flag开启数据。

安全默认及模板可版本化；本机覆盖写入被忽略的 `config/local.toml`。以下仅说明 development/test 的显式装配字段，不启用业务Flag：

```toml
[adapters]
mode = "local_test"
local_root = ".local/identity-spool"

[identity]
password_minimum = 8
active_token_version = "local_v1"
mail_from = "noreply@example.invalid"
mail_base_url = "http://127.0.0.1:8081"
# 可选离线泄露语料，两项必须同时设置：
# password_corpus_path = ".local/offline-corpus.json"
# password_corpus_sha256 = "填入完整文件的64位小写SHA256"
```

Secret只经环境/外部注入，禁止放进TOML、命令参数、日志、截图或提交：

| Secret | 内容与约束 |
|---|---|
| `BBBCM_IDENTITY_TOKEN_KEYS` | JSON对象，键为不重复版本名，值为独立随机32字节的小写hex；包含active及仍需重建的旧版本。最多16版本、输入8192字节，非法/重复版本拒绝启动。 |
| `BBBCM_IDENTITY_RISK_KEY` | 独立随机32字节的小写hex；不可与任一邮件派生密钥相同。邮件装配启用时必填。 |

密钥值不在本手册展示。由既有Secret管理方式注入两个宿主，重启保持相同版本的值；不能每次启动生成替代值。`active_token_version` 必须存在，发件地址限 `example.invalid`，邮件链接限含端口的回环HTTP origin。staging/production拒绝本地模拟装配。配置错误仅输出封闭原因，不回显Secret输入。

本地代码使用64位hex文本作为HMAC key字节串；Web/Worker统一经 `apps/identity_services.rs` 构造，不能在某个宿主单独改成hex解码后的字节。若将来改变编码需新合同/新版本，不能改变旧版本含义。

独立Worker沿用 [引擎手册](engine.md) 与数据库角色配置；SQLite独立Worker须关闭内嵌宿主，禁止双宿主争用。同一份非秘密身份配置与Secret版本集必须注入Web和Worker；Web不借Worker凭据。

## 令牌、邮件、重发与密钥恢复

预认证主体固定2小时；表单之间复用独立HttpOnly预认证及CSRF Cookie，GET不会替换仍有效证明。邮箱验证及新邮箱确认24小时，恢复1小时，普通会话30天绝对/14天空闲，重新认证5分钟。新幂等请求原子作废同账号同目的旧token，写入新token、消息引用与Outbox；同key重放不重复换token/发邮件。`/resend-verification` 接受统一公开结果；已验证、未知、无效账号不泄露差异。

HMAC派生将密钥版本、随机nonce、令牌引用、账号、目的、目标邮箱规范键、期限和签发凭证版本绑定。数据库仅保存校验哈希和非秘密重建上下文；Outbox只含消息引用。邮箱一次性令牌原值仅在签发/消费内存和明确标注的私有本地邮件中出现，不进入Outbox、明文任务表或日志。

邮件通过现有 `MailSink`、`DbMailStore`、Scheduler和持久抑制规则写出 `.local/identity-spool/mail/*.eml`。文件标注 `X-BBBcm-Delivery: local-test-only`，含A00本地品牌；邮件给回环页面地址并提示粘贴令牌，不把token置于URL query。`LocalRecorded`仅表示本地输出，没有真实发送或送达。

1. source始终按不可变上下文重建原正文。消费、过期、重发作废或凭证版本变化不修改旧正文；令牌是否仍有效由消费事务判断。
2. 持久消息/投递记录抑制重复输出；同一规范邮箱复用稳定 `recipient_ref`，投诉/硬退信等规则不被别名展示绕过。营销退订不阻止安全邮件。
3. provider/source暂不可用时保留原event/message ID，由既有Worker重试。重启后重建同一source继续处理；缺失旧key时返回不可用，恢复原版本后重试。不要删除receipt、抑制记录或换事件ID来恢复。
4. 正常轮换先给所有宿主注入新版本并保留旧版本，再切active签新token。旧版挑战也按保留版本重建本地proof。未知提交或正在重试的消息不能靠移除旧key消除。
5. 疑似泄露时先停签、按受信事故流程失效关联未消费令牌，保留脱敏操作引用。本轮没有管理失效/销毁命令，不手工修改正常库来假装完成响应。
6. 仅当消息已不可能加载/重放，且备份恢复也不再需要该版本时，才可退役对应旧key。本轮不设置生产保留天数或自动清理；1/24小时token TTL不能推导为消息/key删除时点。永久丢失/重试耗尽按 [适配器手册](adapters.md) 处理并保留失败事实。

密码重置/变更和新邮箱确认撤销全部旧会话及相应旧凭证令牌，密码与邮箱变更通知共同进入Outbox。登录异常检测与独立触发通知尚未实现，现有login_security模板本身不构成交付。

## 限流、挑战与离线密码筛查

参数版本以DEC-051为准，`RateProtection::approved_v1`复用A05令牌桶：次数为初始容量/长期平均预算，每token补充间隔为“周期÷容量”，不是严格固定窗口。账号、IP、设备、网络四维共同约束；网络IPv4 /24、IPv6 /64。账号别名统一主体；公开失败/限流保持一致的实际Argon2工作量。Redis关闭明确 `NoProtection`，已开启但依赖不可用按高风险合同失败关闭，不退回内存限流。

软挑战使用独立设备/IP桶，两者同时耗尽才触发。挑战绑定预认证、动作、幂等键和实际请求指纹，5分钟、单次；证明与目标命令共同消费。原请求回执可重放，证明不能换动作/主体/密码/邀请码使用。注册/登录挑战同事务登记RiskEvent v1可解释结果，不为恢复伪造新事件类型。

本轮唯一provider是 `LocalChallengeProvider`，仅模拟流程，无真实防机器人效果。proof在私有 `processed/<challenge_id>.json`，页面只显示引用并要求输入本地proof；密码和邀请码重新填写，其他非秘密字段及原幂等键保留。没有provider或provider不可用时失败关闭，不接受客户端自报“已通过”。这些产物不得放进静态目录或提交。

默认 `CommonPasswordScreen` 使用版本化常见弱密码列表，不声称是完整泄露数据库。可选离线JSON需包含 `schema_version:1`、非空 `corpus_version`、`algorithm:"sha256"` 和 `sha256_digests` 数组；Web身份服务启动时核对完整文件SHA256，拒绝缺失、损坏、非普通文件或超过64MiB输入。配置了语料就不能失败退回仅常见密码筛查。不发起外部查询；真实语料来源、更新和正式验证码provider仍留DEC-011/CR-014，合成语料不代表真实语料已获批或生产检查已接入；adapter状态按99/README裁决。

风险只保存用途隔离HMAC信号与封闭动作/原因/规则版本，不保存原始IP、User-Agent、邮箱或邀请码。注册、验证、邀请使用分析事件同业务事务提交，默认将本地来源排除于正式指标；HTTP不能自报排除事实。首次阅读/回复/贡献、组合时序检测及看板留后续。

## 未知提交、敏感日志与验证

HTTP202携带原operation_id时，用同一浏览器原证明通过 `POST /auth/operations` 查询原actor/scope/幂等键，不换key盲重试。公开流程用预认证+CSRF；会话已因退出/改密撤销后，仍仅允许在原绝对期限内查询自己原操作摘要，不恢复普通认证。`Pending/NotObserved`不等于回滚；证明丢失或过期没有本轮管理员救援入口。

页面no-store/no-referrer，邀请页noindex无OG。日志记录封闭路由、生成的关联、状态和耗时，不记录URI/query/表单/Cookie/Secret。只清理已停止宿主、关闭连接且确认归属的本机合成测试目录；不清空非本地库。数据库、WAL/SHM、邮件、challenge proof、Secret和本机配置继续忽略。

```bash
cargo test --locked -p identity --lib
cargo test --locked -p trust -p moderation --lib
cargo test --locked -p bbbcm-web runtime_tests
./scripts/dev-check.sh
```

测试使用真实临时SQLite、实际批准的policy和应用工厂验证注册、会话、邀请、重发、邮件输出、缺旧key失败/恢复、Worker重启、离线语料失败。独立复核、结果和遗留缺口见99 §5.8及其开发证据。不据此宣称真实PG/Redis、完整安全/并发/性能、生产IAM、供应商或正式视觉矩阵通过。
