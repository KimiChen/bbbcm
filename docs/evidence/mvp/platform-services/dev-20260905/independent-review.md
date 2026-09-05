# 独立交叉复核

本记录只反映开发期代码与受控本地测试复核，不是完整安全、性能或生产依赖审计。最终结论：无未关闭的本轮代码阻断。

| 实现范围 | 独立复核 | 发现与收敛 |
|---|---|---|
| Redis 异步合同、TTL/namespace、短租约、限流和唤醒 | analytics_a06、root | 精度截断、连接代际清理、慢 TCP 与取消时间预算修正；内存限流只测试，关闭模式无保护，高风险失败关闭；真实 Redis 合同入口明确 ignored |
| 分析字典、持久投影与重建 | redis_a05、root | 已提交 Outbox 外壳/trace 绑定校验、微秒精度、完整 SQL 字面量及固定 bind 数组；根锁/receipt/聚合/水位共同提交，迟到不被水位跳过，匿名维度无正文 |
| 邮件与本地媒体适配 | analytics_a06、redis_a05、root | 解码前元数据分配预算、祖先符号链接、禁止小图放大绕过像素预算；反馈窗口微秒；已提交回执丢失先去重，待处理新抑制持久化；报告角色最小 schema/列授权 |
| HTTP/健康/状态与日志 | redis_a05、analytics_a06 | PG 公告 schema USAGE、数据库失败与公告空态分离、中性降级文案、C0 转义；请求输入不作为日志关联或无限标签，原始内部错误不输出 |
| trace 归一与子事件继承 | root、adapters_a06 | 生成请求 ID → 实际事务 → 持久 Outbox → Scheduler task-local → sink 子事件的 SQLite 链路断言；保留合法上游 trace，拒绝不合法事件 trace，命令日志过滤非法 task-local |
| 有限指标与事务结果 | root、redis_a05 | 补后端/用途/HTTP/sink 有限分组，取得连接失败和取消分列，明确 commit/rollback/unknown/abandoned；进程 ID 仅重置段元数据，seconds 和 ms 不混算 |
| App/Worker 池与生命周期 | analytics_a06、root | 真实 driver gauges、用途区分、同池 Clone 不重记、Weak 注册不延长生命周期；未接池显式 NotConnected，关闭池可真实为零 |
| Worker 新观测与调度 | analytics_a06、root | 相邻 tick 真实间隔、active guard、取消/失败退出、sink结果/租约错误、RetryExhausted 与普通 retry 区分；不以停用成功伪造投递 |
| 共享应用装配 | adapters_a06、root | local_test 总门开启时 DisabledReplica 正确映射 Disabled，不消耗尝试/入 DLQ；独立/内嵌使用同 scheduler，PG 保持专用 Worker 凭证 |
| Outbox gauges 和操作文档 | redis_a05、analytics_a06 | 事件与 delivery fanout 分开、已完成 delivery 优先、自定义 sink 归 Other；发生时间年龄不冒称提交延迟；PG 同一只读事务的多 SELECT 不冒称单一提交快照，未注册 sink 的 DLQ 文案准确 |

审查后的实际 SQLite 测试包含丢通知/持续唤醒错误后新提交的轮询恢复、取消退出、邮件与对象真实落盘、反馈去重与抑制、disabled audit 重试预算、连接池及真实 task abort。受控模拟和临时 SQLite 结果不代表真实 PG 网络、Redis Lua 原子性、provider 投递或完整并发矩阵。
