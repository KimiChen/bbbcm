# Redis 装配与恢复（A05）

本手册是 [02 §12/17/18](../../plan/02-technical-architecture.md) 的派生操作说明，范围与状态见 [99 §5.6](../../plan/99-traceability.md)。仅本机开发，业务 Flag 保持关闭。Redis 不保存会话、权限、幂等结果、业务事实、审计或 Outbox 的权威副本。

## 配置与装配

现有 `features.redis`、`server.environment` 与 Secret `BBBCM_REDIS_URL` 是唯一装配输入。连接串仅经受控进程环境或 Secrets 注入，不写 TOML、命令参数、日志或提交；`config/local.toml`、数据和本地处理结果保持忽略。

`platform::cache::RedisRuntime::from_config(&Config)` 校验配置并生成共享依赖：

| 配置 | CacheStore | Wakeup | Lease | RateLimiter |
|---|---|---|---|---|
| Redis 关闭 | 有界进程内 LRU | 固定间隔轮询 | 有界内存短租约 | 明确 `CacheError::Disabled`，无保护 |
| Redis 开启 | Redis 共享公开缓存 | Redis Stream 提示 | Redis 令牌短租约 | Redis 原子共享令牌桶 |

启用时构造 Redis client，但延迟连接。Redis 暂时不可用不会阻止进程启动或普通公开读取；运行请求返回脱敏 `Unavailable`，重连由下一次操作触发；按连接代次清除失败连接，旧请求的失败不能删除新连接，清理不阻塞超时返回。一次普通网络操作含等待连接最多 2 秒，不在适配器内自动重试写命令，避免提交回执丢失造成重复扣减等不确定效果。读取、限流及短租约调用均为 `BoxFuture<Result<…>>`，可通过 `Arc<dyn Trait>` 注入，不能把依赖错误当成功。

`check_rate_limit` 用合成 `HighRisk` / `PublicRead` 上下文集中映射故障：高危不可用返回错误，公开读不可用返回 `OriginFallback`，关闭返回 `NoProtection`。`read_public` 实际调用注入的主库读取函数，并返回缓存命中、回源和缓存写失败的独立状态；主库读取失败原样向上传播，不伪造值。

生产限流不使用进程内替代。Redis 关闭时没有注册、登录、找回等速率保护，SQLite 体验模式只适用于本机体验；现有配置校验继续拒绝其公开部署。Redis 开启但不可用时，高风险限流操作返回不可用并失败关闭；普通公开读取回主库。主库亦不可用时，遵守架构的有限短期公开缓存/静态故障页边界，不从过期或未经权限过滤的数据拼接结果。

新增 `redis` 1.6.0 是实现实际 Redis 协议、异步 Tokio 与 TLS 的必要依赖；禁用默认功能，仅开启 Tokio/Rustls 和证书根支持，复用已有 Tokio/UUID/SHA-256。没有引入第二个数据库层或 `platform-*` 横向依赖。[redis-rs 官方文档](https://docs.rs/redis/1.6.0/redis/)说明了多路复用连接及 Tokio/TLS 能力。

## 受控键与公开缓存

所有键都由 `Namespace(Environment)` 生成 `bcm:<environment>:v1:` 前缀，环境只允许已有四值枚举。业务调用不能提供完整 Redis 键、清理 pattern 或环境字符串。实体 kind 为受信代码/Outbox 生产者提供的机器名，实体 ID 只接受正整数或 UUID；限流维度与租约 kind 验证长度、字符，身份部分在入键前 SHA-256。键摘要仍不适合作日志或指标标签。

公开片段键为 `cache:<kind>:<id摘要>:public:<version>`。调用方必须先取得受信的当前实体版本与可见性；`PublicCacheKey::new` 拒绝 `Private` 或零版本。私有、个性化及登录态页面不得通过此共享缓存。删除、封禁、权限收紧、后台及高危操作仍回查主库；`Public` 是调用方承担的授权判断，不是 Redis 帮忙验证了权限。

`DEFAULT_PUBLIC_TTL` 为 30 秒，写入允许整毫秒精度的 1 毫秒至 30 秒且每值最多 1 MiB；这个上限是适配器资源防护，不是生产容量配额。内存实现按实际访问次序 LRU 淘汰。实体失效删除其全部版本，重复执行没有额外副作用。

`CacheInvalidationSink` 注册名和 `event_type` 均为 `cache.invalidate`，资源来自事件 `aggregate_type` / `aggregate_id`，载荷不需要缓存正文。领域变更必须在业务事务内持久化目标 sink，再由现有 Worker 消费已提交 Outbox；如需提交后立即失效，也只能在提交明确成功分支调用，Outbox 仍负责可靠重试。回滚、未知提交或未提交事件不得提前失效。依赖不可用映射为 sink 可重试失败，非法事件为永久失败；领域真实生产接线随后续包交付。

## 唤醒与短租约

唤醒只用 `bcm:<environment>:v1:wakeup:outbox` 一个 Stream，`XADD MAXLEN 1` 写入无业务载荷的提示。`XREAD BLOCK` 从当前游标等待新提示，使用专用连接，避免阻塞缓存、限流或续租连接；超时和取消会结束等待。通知可能在等待开始前丢失，连接中断或 Redis 关闭时仍依赖 Worker 的固定间隔数据库轮询。没有 Redis consumer group、权威消费去重或业务锁状态；数据库 Outbox 租约和 `(sink,event_id)` 才是恢复事实。[XREAD](https://redis.io/docs/latest/commands/xread/) 与 [XADD](https://redis.io/docs/latest/commands/xadd/) 命令行为见官方文档。

短租约通过 `SET NX PX` 获取，每次持有者令牌使用新的 UUID；Debug 不显示令牌。续租和释放使用单键 Lua 原子校验当前令牌及正 TTL，到期租约和旧令牌均拒绝，旧持有者不能操作随后获得的新租约。内存实现遵守相同令牌和到期语义；Redis 锁仅防重复执行或缓存击穿，业务正确性仍依赖主库锁与约束。

限流策略由受信调用方提供容量与整毫秒精度的单令牌补充周期，平台不猜注册/登录等产品阈值。Redis Lua 使用服务端 `TIME` 计算补充，在一次原子执行中读取、扣减、写回和设置 TTL；多个实例共享同一桶。同一个维度下改变未过期桶的策略会拒绝，不能以改参数重置桶；服务端时钟后退不会提前补充或提前过期。拒绝结果提供 `retry_after`。

## 观测与清理

`RedisBackend::health()` 实际执行 PING；`metrics()` 只返回固定字段计数：缓存命中/未中/错误/失效、租约取得/冲突/续租/丢失/错误、唤醒通知/收到/超时/错误、限流允许/拒绝/错误及依赖错误。计数是真实进程观测，Redis 关闭时不创建伪造 Redis 零指标；应用观测层负责把缺失或未接入分类公开为适当状态。原始键、令牌、事件 ID、Secret、连接串不进入指标和底层错误。

`clear_namespace()` 是显式操作能力，不自动运行，也没有未经鉴权的 HTTP 清理入口。它只扫描已配置当前环境的前缀并分批 `UNLINK`，再次验证扫描回包属于该前缀；不使用 `FLUSHDB`、`FLUSHALL` 或 `KEYS`，不接受用户自定义 pattern。清理不会改主库数据；操作前暂停相关可重建写入可获得稳定清理结果，并发生产会重新出现缓存。此方法会重置当前环境限流状态、短租约和提示，运行操作须按当前环境的受控运维边界进行；本轮没有实际清理生产或其他实例。

恢复时直接由主库重建公开缓存并恢复轮询；不导入旧 Redis 备份作为权限、幂等或 Outbox 事实。限流状态丢失会降低安全姿态，不能把清空当无风险维护。

## 验证入口

开发期运行 `cargo test -p platform cache::`、适用 Worker/SQLite 测试和仓库 `./scripts/dev-check.sh`。内存合同、配置装配、禁用/不可用区分和关闭 TCP 对端测试不等于真实 Redis Lua 原子性验证。

真实合同入口默认 ignored，仅用于显式准备的可丢弃本机 Redis 8 实例。按 `DEC-063`，服务端只锁**大版本 8.x**：任何 Redis 8 补丁线都可用于该合同，不要求与 `supply-chain/lock.json` 的 8.2.9-bookworm 小版本一致——合同验证的命令与 Lua 原子性语义在 8.x 内无差异，把证据绑定到单个补丁号只会让它随上游发版失效。锁定 manifest 仍是镜像构建与 SBOM 溯源的事实源，不因此放宽；无论用哪种方式起实例，都不连接生产。先用已有供应链流程准备独立实例，绑定回环地址；测试前通过本机受控环境设置 `REDIS_CONTRACT_URL`（只允许字面回环 IP，不打印连接值）和 `REDIS_CONTRACT_ISOLATED=yes`，再执行：

```bash
cargo test -p platform --test redis_contract -- --ignored --exact redis8_contract_atomicity_namespace_and_recovery
```

测试先核 Redis 主版本为 8，再覆盖真实缓存 TTL/版本失效、令牌短租约及到期拒绝、64 个任务争用同一限流桶、Stream 提示/丢失/取消、namespace 清理不触及另一环境与无关键、从合成源重建。清理会删除实例中整个 `bcm:test:v1:` 前缀，故仅可使用显式隔离的可丢弃实例；测试结束移除其创建的其他合成哨兵。Redis 网络中断全矩阵、跨进程/多主机并发、安全、性能、真实 PG 及视觉等完整验证仍在 E01/E02/E04。本轮若没有运行该入口，必须记为未执行。
