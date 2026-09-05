# 容量与性能派生模型：WP-M0-05

本文件由 [99](../../plan/99-traceability.md) 登记，是 [01 §7.5/8.3](../../plan/01-product-requirements.md)、[02](../../plan/02-technical-architecture.md)、[90 §6](../../plan/90-quality-release.md) 的派生设计，不建立计划或状态事实源。工作包编号依据 DEC-036；模型实现与复核由 Codex 负责，用户是项目负责人和最终批准人。依赖 M0-01、M0-03，沿用 [M0-04 数据设计](2026-09-database-foundation.md) 的索引、事务、Outbox、追加审计和保留边界。

本轮仅提供模型、负载档案、确定性生成方式和测量合同。没有百万业务数据灌入、数据库容量测量、压测、地区探测或性能达标证据。没有新业务表、Flag、权限、Redis、真实 provider 或发布行为；A05 接指标，E02 执行性能与容量硬化。当前状态、检查与剩余依赖只在 99 登记。DEC-020/039 保持开发期最小集，不为本包增加独立构建或全量扫描。

## 1. 目标、假设与实测严格分栏

[capacity-baseline-v1.json](../performance/capacity-baseline-v1.json) 是本文的机器可读实验输入；`targets` 引用计划，`assumptions` 是可修订的建模选择，`pending_production_values` 留空，`measurement.executed=false`。JSON 和本文都不能覆盖计划。模型变化须同时复核本文、生成器、测试并在 99 登记版本；生产决定仍先走 06。

| 类型 | 数值或边界 | 解释 |
|---|---|---|
| 计划容量目标 | 10,000 注册用户；300 峰值在线；1,000,000 主题与回复 | 百万是主题和回复的合计，不能各建一百万；在线是会话规模，不是连接池大小 |
| 计划负载起点 | 200 public read/s + 20 write/s | 同一个混合窗口；是 offered 到达率起点，不是最大吞吐、常年平均或承诺 |
| 计划延迟目标 | 公开读 p95 ≤400ms；写 p95 ≤800ms；SSR 首屏服务端响应 p95 ≤800ms | 地区、端点、缓存层和失败必须分别报告；SSR 边界见 §4 |
| 模型数据假设 | 100,000 主题 + 900,000 回复 | 1:9 是负载档案选择，不是产品强制比例 |
| 模型实验窗口 | 300s 预热 + 900s 测量，独立重复 3 次 | E02 冻结实验清单；不把此选择说成生产维护窗口、SLA 或获批统计周期 |
| 实测 | 未执行，全部结果为 null | 单元测试验证计数与计算，不证明 p95、吞吐或磁盘占用 |
| Pending | VPS 地域/CPU/RAM/卷/IOPS、连接池预算、配额/费用/扩容阈值、主要用户具体测点、WAL/备份/Outbox/DLQ/审计保留与 RPO/RTO | DEC-004-B/013、M0-04 保留矩阵与相应责任人未批准；不得填猜测默认，不购买、不部署、不自动清理 |

普通幂等响应 30 天来自 01 §7.3，不能将此期限推广到 Outbox、sink 去重或审计。合成历史跨度 365 天只表示测试数据年龄分布，不能作为任何数据保留期限。

## 2. 负载档案

`scenario_id` 是实验标签，不是已经存在的路由。E02 必须把每项映射到届时实现的服务端入口、权限上下文和响应校验；不能拿 A00 静态夹具冒充业务端点。首次硬化同时执行混合负载和逐场景分解，混合 p95 不能隐藏单条常用接口超标。

| 读场景 | 读比例 / 到达率 | 内容与校验 |
|---|---|---|
| `topic_detail` | 45% / 90s⁻¹ | 公共主题首屏、楼层 keyset 分页；以可公开集合中按固定种子选出的10%主题承载80%详情读，余下20%均匀分布 |
| `forum_latest` | 20% / 40s⁻¹ | 四版块最新回复流、最新主题流；分别按01 §6.2复合索引路径取样 |
| `home_feed` | 20% / 40s⁻¹ | 首页公共聚合和首屏 SSR；权限/隐藏内容不得出现在缓存或输出 |
| `content_search` | 10% / 20s⁻¹ | PG中文正文、标题模糊、零结果和高频词；读取当期词典版本与权限过滤；SQLite标题降级另报，不与完整搜索混算 |
| `public_profile` | 5% / 10s⁻¹ | 仅公开资料与公开内容；不引入真实身份、邮件或个性化响应缓存 |

前三项 SSR 是同一读流量的子集（170s⁻¹），不能再加170 read/s；端点必须同时满足适用的公开读与 SSR 指标，不能用800ms替代公开读400ms。SSR以外的页面以后若也渲染HTML，E02需扩充映射并保留模型版本。

| 写场景 | 写比例 / 到达率 | 必须完成的逻辑命令 |
|---|---|---|
| `create_topic` | 10% / 2s⁻¹ | 合成已授权 actor、版块/版本检查、主题与权威搜索状态、幂等、Outbox同事务 |
| `create_reply` | 70% / 14s⁻¹ | 固定主题锁序、专用楼层计数器、版本和同事务事实；常规分布与单热点竞争分开 |
| `edit_content` | 10% / 2s⁻¹ | 当前版本条件更新；校验修订和权限传播，不用覆盖写提高吞吐 |
| `bookmark` | 5% / 1s⁻¹ | 业务唯一键与幂等；重复命令不增加事实 |
| `report` | 5% / 1s⁻¹ | 已授权举报、必要证据摘要和审计；高风险限流失败关闭 |

常规成功负载使用满足已实施业务门槛的合成测试上下文；不能用大量预期403/429/409凑足20 write/s，也不能为压测关闭鉴权、限流或审计。版本冲突、同/异载荷幂等重放、锁等待、提交未知、Redis失败关闭分别作为正确性/竞争情形，报原始操作数、实际尝试数和结果，不能混入成功延迟并宣称达标。当前生成器不创建这些账号或鉴权环境，领域测试适配由后续B/C/D与E02补齐。

到达方式为开环固定速率：按计划时刻调度220个逻辑操作/s，预先生成场景序列，按合计比例的周期排列并用固定种子打散。调度器记录每个应发时刻，不等上个请求返回再调下个。负载机背压、并发上限或资源不足造成的未发请求必须计为 `not_started`；不得降低实际到达率来保住p95。300个合成在线会话在整个窗口存活，220/300≈0.733个操作/会话/s只是平均分配值；不据此设300个同时HTTP请求、数据库连接或固定think time。

每个900s窗口应有180,000次读、18,000次写。此档案每秒新增2主题+14回复，测量期将新增14,400条内容（预热另增4,800），应记录初始/结束精确规模。独立重复应从同一已验证的隔离测试快照或重新生成的数据开始；只允许对显式临时测试库操作，不清空非本地数据库。附加的热点、Redis不可用、Worker停用/积压恢复、搜索重建与日后活动3倍峰值应各有run_id；它们不扩大本轮实现范围或混入常规基线。

## 3. 数据规模、生成与容量计算

模型分布如下，均为假设，E02可依据后续获批的匿名统计另建版本。四版块名称来自CAT-000/001，但比例并非已批准运营配额。

| 维度 | v1 生成方式 |
|---|---|
| 实体关系 | 10,000无登录能力的合成用户引用、100,000主题、900,000回复；用户引用为逻辑fixture key，未来导入器通过稳定键与平台主键回读建立映射，不直接充当自增主键 |
| 四版块主题分布 | 问答/实战40%、AI25%、域名与主机25%、站务10%；回复继承主题版块 |
| 权限测试分层 | 主题90% public、8% restricted_fixture、2% hidden_fixture；后两项是测试标签，需映射届时真实权限与状态，不创建新产品枚举。回复继承；公开读候选严格排除后两项 |
| 热度与楼层 | 确定性选择10%主题承载60%回复，其余回复均匀分配到剩余主题；主题按序逻辑引用，热集合与正文/版块/可见性分布使用不同随机域。回复从2楼顺次递增（主题为1楼） |
| 主题正文UTF-8字节 | 70%为256B、25%为2048B、5%为16384B；标题96B |
| 回复正文UTF-8字节 | 70%为128B、25%为512B、5%为4096B |
| 年龄与版本 | UTC固定基点2026-01-01T00:00:00.000000Z；主题在此前365天按固定排列均匀分布，回复位于父主题与基点之间；固定27字符微秒时间，初始version=1 |
| 内容与查询语料 | 只含合成中文词、Rust/AI等词和按seed/逻辑ID/块号产生的SHA-256摘要；UTF-8按完整码点截断并补ASCII。无真实用户内容、凭证、邮箱或provider资料 |

均匀年龄、短/中/长正文和简化语料不代表真实语义相关性、压缩率或用户行为。E02需另外固定经审查的中文搜索相关性/权限/修订/删除样本、查询选择率和零结果集合；不能凭合成摘要语料宣称pg_jieba相关性通过。权限数据只测试过滤，不能授予尚未批准的生产IAM。

[capacity_model.py](../../scripts/capacity_model.py) 提供本机算法入口：

```bash
python3 scripts/capacity_model.py check
python3 scripts/test_capacity_model.py
python3 scripts/capacity_model.py sample --content-rows 1000 > /tmp/bbbcm-capacity-preview.ndjson
```

`check`仅计算百万规模计数与字节总和；`sample`仅向stdout输出NDJSON逻辑行，内容行数必须为100的倍数且位于100..10000。示例生成10用户+100主题+900回复及一个manifest，不接数据库、HTTP、Redis或provider。预览文件属于可重复输出，留在忽略目录或明确临时目录，不提交。命令请求百万行会在输出前失败；本轮不会实现能够误灌百万业务数据的导入入口。

全规模生成方式与预览使用同一版本算法：先以目标数生成用户/主题逻辑引用，最大余数法使每项分布精确合计；按SHA-256种子派生互素乘数与偏移，对每个属性作仿射双射排列，避免抽样取模造成重复和遗漏；先生成主题，再按冷热集合循环分配回复并维护每主题楼层；内容按上述字节档案逐行输出，最后生成精确计数、输入/输出SHA-256和关系校验报告。百万规模是100,000主题和900,000回复，算法只需维护主题映射/楼层计数，不需把全部正文驻留内存。

E02在后续业务表与服务已交付后接入专用测试适配器并显式解除预览规模限制，按本版本算法输出完整数据；经现有迁移runner建隔离库，以受信SchemaRenderer、bind、Repository/同事务与平台主键回读写入，禁止手写物理前缀、跳过唯一约束、直接写身份授权或改已应用迁移。适配器须声明哪些行仅作历史夹具及为何不触发真实外部副作用，并用本地sink验证必要事件一致性。导入前后核对用户/主题/回复数、关系、楼层唯一、可见性、版本、必要索引和已注册Flag全关；这是生成方式的后续接入，不是本轮已实现的导入能力。

容量计算只使用可解释单位：B为字节；MB=10⁶B；MiB=2²⁰B；GiB=2³⁰B。主题正文期望1510.4B、回复422.4B，在精确档位计数下合计531,200,000B（506.591796875MiB）；主题标题另9,600,000B。它只是合成逻辑文本输入量，**不能称为PG磁盘下限**：TOAST/压缩可能减少文本物理量，行头、索引、修订等又增加物理量。

实际规划容量必须在E02测量后逐项相加：表heap/TOAST及其索引、业务/搜索索引、幂等、Outbox/投递去重/DLQ、追加审计、临时排序/维护空间、WAL，以及独立故障域备份。每项以其实际表/索引/版本与窗口记录，不用未经测量的“索引占正文x倍”或VPS默认规格。

- 连接并行占用的诊断估计：`C_mean ≈ Σ(λ_i × mean_db_hold_seconds_i)`，另加Worker/后台读写；hold是借出连接到归还（含锁等待），pool acquire等待单独测。Little定律使用稳定窗口的均值；不能用p95、总在线数或HTTP时长直接定连接池。实际预算需计Web/Worker各进程总池、维护/迁移连接与保留余量，由DEC-004-B批准。
- WAL生成率：同窗口计数差分`Δwal_bytes / Δseconds`；峰值、归档/检查点行为和磁盘实测分别记录。WAL/备份保留期未定，只给`rate × duration`敏感性公式，不据此实施清理或永久保留。
- 普通幂等记录数量估计：`λ_new_unique_commands × 86400 × 30 × duty_factor`。若把20/s全视作全新的普通幂等命令且全天持续，代数上为51,840,000条/30天；这是上界情景假设，不是预期日活或已批准容量。实际重放率、昼夜峰谷、响应大小和业务唯一事实另测，过期响应不能清除业务唯一事实。
- Outbox：`λ_events=Σ(λ_command × events_per_command)`；`λ_deliveries=Σ(λ_event × enabled_sink_fanout)`，两者单位不同。稳定性观察`μ_successful_deliveries > λ_deliveries`及队列年龄，积压排空估计`B/(μ−λ)`仅当μ>λ成立。重试增加尝试数但不能增加成功事实；暂停、DLQ或待重放行分开，租约/保留/告警阈值不得由本模型猜定。

## 4. 地区、设备、缓存与时间边界

每组run_id必须有后端、版本、数据哈希、场景、地区、设备、网络、缓存、窗口和资源配置。缺字段只允许登记实验缺口，不得支撑正式基线或Production Approved。

地区分层为：①同VPS/隔离测试网内的origin诊断；②中国大陆代表测点；③跨境中文用户代表测点。②③来自DEC-006-A目标人群，具体城市/网络运营商/负载机与目标地域尚待E02实验冻结及DEC-004-B生产批准；本轮不选或伪造。客户端记录实际城市/地区标签、网络类型/ASN或已脱敏网络类别、直连/CDN路径、网络整形设置与测得RTT/丢包。不同地区不混算p95；origin诊断不能代替用户地区结果。

设备分层为桌面和移动：桌面原生浏览器、移动真实设备优先；可增加明确标注的浏览器模拟移动实验。每组记录设备型号/CPU/RAM/OS、浏览器精确版本、viewport/DPR、CPU与网络是否整形及实际值、HTTP版本和连接复用策略。模拟设备不是实际手机证据。此处不填未测的CPU或带宽值；E02运行清单冻结后填写。SSR首字节门槛不随桌面/移动改变，压缩HTML体积、完整下载与LCP另报，HTML预算按01 §8.3待硬化测量。

| 口径 | 起止边界与分母 |
|---|---|
| 客户端公开读/写响应 | 逻辑操作实际request start至响应体读完；包含网络、代理、队列、业务、序列化及返回传输；另记录预定start到完成的调度含时延，防负载机排队被隐藏 |
| SSR 首屏服务端响应 | 地区浏览器Navigation/request start至`responseStart`首字节，即TTFB；新连接包含其DNS/TCP/TLS，复用连接按实际；重定向链是否存在单独标记。不得把DOMContentLoaded、load或LCP当800ms判定值 |
| Origin SSR span | Web接收请求至首字节交出，拆解鉴权/池等待/查询/模板渲染；这是诊断指标，不替代含网络的地区TTFB |
| 写逻辑完成 | 确定提交及规定响应；202提交未知计`indeterminate`，报告到受控查询确定终态的额外时长。不能把收到202当成功写完成，也不能无鉴权查询或盲目重放 |
| 原始采样 | 每个逻辑operation有scheduled/started/first_byte/completed单调时钟偏移与outcome，UTC窗口只用于跨组件关联；attempt编号独立。时长用同一进程单调时钟差，跨主机绝对钟不能直接相减 |
| p95计算 | 分层内排序后取第`ceil(0.95×n)`项（nearest-rank），无样本为null；同时报n、min/max、p50/p99及完成/错误/超时/未发数量。禁止平均分钟p95或不同地区p95；重复实验各报，合并时拼接同条件原始样本 |

实验预热不纳入稳态p95，但必须保留其完整窗口和失败计数。测量按预定到达时间位于`[T0,T1)`的操作归属；T1之后等其完成或显式超时截止并记录截止值，不能因结束时仍在运行而丢样本。每个run显式记录客户端超时配置、调度落后与丢弃阈值；未冻结时不填成功结果。900s×3是模型假设，冷热矩阵不能把预热300s照搬成“冷”测量。

错误响应的实际完成时长、成功时长分别计算p95，timeout只记录已等待下界和censored标识，未发/取消/未知提交没有虚构完成时长。另可报告保守的offered score：将所有非确定成功操作（包括快速错误）视为+∞参与排列；该值无界时以null加`unbounded=true`表示。这是防漏样本诊断，**不是物理延迟分位数或新的获批错误预算**。`summarize_samples`的小测试实现这一子集算法，始终返回`performance_pass=null`；真实端到端采样器由E02接入，不能据其空接口或合成数字判定达标。

| 缓存实验 | 操作与证据 |
|---|---|
| 应用/Redis热、边缘热 | 使用固定数据和请求种子预热；记录各层命中/未命中/绕过计数、TTL、实体版本/可见性键，命中率由实际观测给出 |
| 应用冷，数据库已热 | 对专属测试namespace清空可重建缓存或重启隔离实例；立刻单独记录首次访问集合与后续恢复，不用稳态窗口冒称冷缓存；禁止FLUSHDB |
| 数据库冷诊断 | 在专用测试实例确认重启/独立卷等过程，分别说明PG shared buffers和OS页缓存是否真的冷；仅重启PG不能声称OS缓存冷。不能为此在生产清页缓存 |
| Redis未启用/不可用 | 公开读回源；高风险限流写失败关闭按DEC-024另报。`features.redis=false`体验模式无速率保护，仅本机体验；不拿失败关闭请求凑正常写吞吐 |
| 直连origin / CDN | 同请求档案分别报告，边缘命中、应用命中、数据库热度三者独立；本轮无Cloudflare或生产访问 |
| 个性化/私有/后台 | 不共享缓存；权限收紧、隐藏/删除与高危路径回查主库，不能用缓存放宽正确性；≤60s传播另按DEC-030测试 |

## 5. A05 指标接入合同

下列为语义字典和建议度量键，尚未装配exporter/Redis/trace provider；A05将其接入现有共享合同，不增platform-*横向依赖。指标以秒、字节、计数为基本单位，报告转ms/MiB；累计counter只在同实例同重置区间差分，重启/统计reset必须切段，缺测为null而非0。PG专用系统统计读取只放受控平台适配器，不进HTTP Handler或普通Repository的可移植SQL。

| 语义键 / 类型 / 单位 | 起点与分组 | 用途与边界 |
|---|---|---|
| `http_duration_seconds` / histogram / s | 实际HTTP完整响应；backend、route模板、method、status_class、cache_state | 客户端完整采样为门槛主入口；服务端histogram报桶边界，不能用粗桶插值替代边界附近的原始p95 |
| `ssr_first_byte_seconds` / histogram / s | 地区客户端首字节；origin span另用组件标签 | 拆解网络、队列、模板；完整HTML/LCP分开 |
| `db_pool_connections` / gauge / count | 每后端、Web/Worker用途的open/idle/in_use/max | 汇总所有进程池和维护预留；不能仅看Web池 |
| `db_pool_acquire_seconds` / histogram / s；`db_pool_acquire_failures_total` / counter | acquire前至取得或失败 | 连接耗尽、超时与请求延迟关联，失败不能当0秒 |
| `db_connection_hold_seconds` / histogram / s | checkout至归还；purpose | 用均值估算并行占用；不要与acquire重复相加 |
| `db_transaction_seconds` / histogram / s；`db_transaction_attempts_total` / counter | BEGIN至COMMIT/ROLLBACK；backend、outcome、受控error_class | 分开确定成功/回滚/提交未知；每个逻辑命令重试次数，唯一冲突不进事务重试预算 |
| `db_lock_wait_seconds` / histogram / s；`db_lock_waiters` / gauge；`db_deadlocks_total` / counter | 应用锁请求与PG等待事件/锁持有时长 | PG lock_timeout=500ms；SQLite busy_timeout=5000ms。busy/deadlock/serialization/unique分别分类，不能改参数掩盖竞争 |
| `db_wal_bytes_total` / counter / B；`db_wal_pending_bytes` / gauge / B | PG实际WAL生成、归档待处理量；统计reset单列 | 同窗B/s、checkpoint耗时/频度、fsync/卷延迟；不推断备份保留已批准 |
| `db_dead_tuples` / gauge / count；`db_autovacuum_runs_total` / counter | PG每受信表dead/live估计、vacuum/analyze次数/耗时和最后成功时间 | 同时看最长事务年龄、事务ID年龄、n_mod_since_analyze；统计估计不是精确计数，不用仅dead ratio作自动清理阈值 |
| `db_relation_bytes` / gauge / B；`db_index_access_total` / counter | 表/TOAST/索引尺寸、scan/read/fetch计数；逻辑对象名 | heap、TOAST及其索引避免双算；对应01 §6.2查询样本、执行计划/行数/排序spill；核心topics/posts不提前分区 |
| `outbox_events` / gauge / count；`outbox_deliveries` / gauge / count | ready/leased/retry/dlq/paused/completed分别报；sink白名单 | 事件与(sink,event)投递数分开；停用的副作用不报成功 |
| `outbox_oldest_pending_seconds` / gauge / s；`outbox_commit_to_sink_seconds` / histogram / s | 最老未终态事件创建时刻、提交至本地sink确定完成 | 无pending时0，读取失败时缺测；事件发生时间不是数据库提交时间，无法精确取commit时必须标明observed近似边界 |
| `outbox_claims_total`、`outbox_lease_losses_total`、`outbox_retries_total`、`outbox_dlq_total` / counters | sink、backend、outcome；claim/续租/旧持有者拒绝/到期回收 | 观察积压与重放是否扩大重复；外部副作用不因数据库去重被称exactly-once |
| `worker_poll_seconds` / histogram；`worker_active_jobs` / gauge | 轮询周期实际间隔、运行中任务、取消/退出、重启恢复 | A05未接Redis时轮询；SQLite独立/内嵌模式互斥，不把两个宿主并跑当吞吐技巧 |
| `audit_append_failures_total` / counter | Audit/SensitiveAccess类型、受控失败类 | 高危审计失败必须回滚，敏感读取授权与审计未提交不返回数据；无payload标签 |

标签只允许低基数枚举：backend、purpose、route模板、scenario_id、受信逻辑表、注册sink、有限outcome/cache_state。禁止actor_id、operation/event/request ID、幂等键、原URL/query、IP、正文、secret/C3进入指标标签。run_id和数据哈希放实验manifest；详细关联以本地脱敏trace证据管理，不将百万实体ID扩成指标。申请PG统计可读权限另走专用最小授权；不能为了采指标给应用migrator权限或审计历史修改权限。

## 6. E02 接收清单与本轮验证

E02的每次实验至少保存以下内容至批准证据位置；仓库只保留适合公开的manifest和摘要，原始网络信息/配置/数据库不入库。未批准的供应商、保留期限与费用仍按M0-06承接。

| 接口 | 必填内容 |
|---|---|
| 输入manifest | model_version、源提交/工作树摘要、模型与生成器SHA-256、seed、全规模/分布/索引/数据集哈希、合成权限上下文说明、Flag快照（生产保持关闭）、后端/扩展/词典版本 |
| 运行manifest | run_id、实际硬件/卷/OS/数据库/Redis版本、所有池/Worker/lease/重试/超时设置、地区/设备/网络/CDN、冷热操作、预热/测量/收尾UTC、单调钟精度与同步说明 |
| 请求原始行 | run/scenario、受控逻辑操作关联、attempt、scheduled/started/first_byte/completed偏移、确定outcome、status、cache层状态、脱敏响应校验；超时下界、未发、取消、提交未知各自保留 |
| 系统时间序列 | 上述A05指标，采样频率、counter reset、缺测区间和统计源版本；开始/结束尺寸、连接池、WAL、vacuum、索引与Outbox积压 |
| 结果 | 各地区/设备/场景/缓存/backend逐组样本数、offered/started/completed/成功吞吐、各outcome计数、原始分位数、调度延迟；混合结果和分解结果并列、失败完整保留 |
| 复核 | 数据一致性、没有跳过鉴权/事务/审计、负载机未成为瓶颈、窗口完整、版本/参数一致、无敏感数据；规格未批准只能形成实验结果，不能支持Production Approved |

开发期以 `python3 scripts/test_capacity_model.py` 验证目标/单位、精确比例与最大余数、1000内容小样本引用/楼层/时间/UTF-8长度、seed可复现和变化、目标漂移/擅填Pending拒绝、CLI百万生成拒绝、nearest-rank与快错/超时/未发不伪造成功。其余实际SQLite、Rust、A00/A01回归和diff/暂存逐文件审查由本轮主任务的 `./scripts/dev-check.sh` 执行并登记99。本包不修改该脚本、不运行额外构建。

留待硬化的工作包括真实PostgreSQL双前缀与Redis采集、完整业务数据导入、实际设备/地区TTFB、Worker并发与重放恢复、百万搜索重建/相关性、磁盘/WAL/vacuum/索引与池测量、容量拐点、性能/安全/视觉全矩阵。模型交付和小样本测试不授予任何产品需求Implemented、Verified、阶段或发布门禁。
