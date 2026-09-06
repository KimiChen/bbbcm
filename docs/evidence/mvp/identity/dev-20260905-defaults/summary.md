# DEC-051 身份默认规则与正常宿主接线开发证据

日期：2026-09-05（America/Los_Angeles）。决策先以`18808a7`提交，令牌桶语义另以`44cb95b`澄清；实现版本为本证据所在提交。此前DEC-050与受控切片分别为`236ba57`、`24360bf`。精确范围/逐需求状态见[99 §5.8](../../../../../plan/99-traceability.md)，配置/恢复见[身份手册](../../../../development/identity.md)。用户负责产品及最终批准，Codex负责本机实现、交叉复核、开发验证与提交。

## 本轮交付

- 正式用户名/邮箱/简介/期限策略；验证邮箱授邀请资格，保持原pending/T0/T1等级。邀请默认3天、整数1–7天、UTC每日5与未用5独立约束，撤销/消费/过期释放未用库存。
- 预认证/CSRF双HttpOnly Cookie固定2小时复用，验证邮件重发、同用途新令牌原子替换与原key幂等重放；真实注册/验证/邀请事务事件继续接入A06。
- 四维A05令牌桶和设备/IP组合软挑战；challenge事实、注册/登录挑战风险记录同事务去重，proof只存哈希、绑定主体/动作/完整请求、5分钟单次、与业务共同消费。
- 版本化常见弱密码拒绝列表和校验完整文件SHA256的可选离线语料；配置后文件缺失/损坏拒绝Web身份服务启动，实际语料/真实provider后续。
- 新增0008挑战PG/SQLite配对迁移、受信注册及最小app ACL；0001–0007原文件未改，MySQL仍占位。无角色/账号/政策/Flag开启种子。
- 正常Web、内嵌/独立Worker共享版本化Secret与身份邮件source；完整Secret缺失时不装配可写身份服务。实际应用工厂测试验证本地邮件、失去旧key失败、恢复旧版本后重试、Worker重建和消费后正文保持一致。
- SSR复用本地冻结组件，新增重发入口、邀请天数及LOCAL TEST挑战页面；观测使用封闭路由分类，无原始路径、Cookie、密码、邮箱或令牌日志。

B02按README §6与04的adapter交付定义为Implemented；真实真人验证、防机器人效果、真实邮件及语料来源不在该状态声明内，生产业务Flag关闭。B01因AUTH-003内容实际调用、INVITE-006按信任调额仍In progress；A02/A06/B04/B06均保留整包缺口。USER-004/005/007完成开发交付，USER-001只有真实基础资料。无Verified或阶段/发布门禁。

## 验证与复核

| 检查 | 结果 |
|---|---|
| `./scripts/dev-check.sh` 最终r5 | 通过：fmt、全workspace/all-target Clippy `-D warnings`、241测试通过/0失败/1真实Redis测试忽略；真实临时SQLite迁移0001–0008和74条HTTP冒烟；diff及handler SQL静态守卫通过。 |
| identity | 46项通过，其中HTTP17、rate4、正式policy3、approved邀请1、challenge4、重发2、密码筛查1；原core7/settings7回归通过。 |
| Web宿主 | 22项通过，包含实际factory注册/验证/恢复/改邮箱、Worker缺key重试/恢复、完整与缺身份配置对照3项。 |
| 信任/政策/平台 | trust4、moderation2；platform76、platform-db62及池观测1项；SQL lint8、Flag/迁移/ACL/Worker既有回归随全仓通过。 |
| A00 | `python3 scripts/asset_freeze.py`通过，原冻结源哈希在既有URL helper归一化后未变化；未宣称像素验收。 |
| 敏感日志隔离 | 修复后Web默认并行22项连续8轮通过；增加跨无subscriber线程首次注册探针，保留关联ID、完成日志、敏感值不出现、响应/指标脱敏与全部JSON解析断言。生产middleware未改。 |

独立代理互审认证、HTTP/限流、策略/信任/邀请事务；root独立复核应用装配、Secret、邮件重建、迁移/ACL、SSR、观测及状态。发现并修复：私有账号hard桶与公开soft挑战响应差异；过期proof在两类硬限流结果下不同响应；缺身份材料仍能写入的装配边界；旧版本挑战proof重建；`8→b`品牌反冒充变体与非法本地发件地址。最终没有未处理的代码审查阻断。

失败历史如实保留：早期宿主测试夹具误写不存在的邀请version列；HTTP固定UTC日期跨天及保留名状态断言已修正。最终最小集r1/r2停在CLI首次迁移/重复跳过的旧7份迁移断言，r4停在Flag测试两处旧迁移数组，均更新为8份；r3暴露tracing-core单dispatcher兴趣缓存与并行无subscriber测试的隔离缺陷。修复只影响测试捕获，所有敏感日志断言保留，r5全通过。作者早期失败与安全发现不作为通过证据。

详细机器可读计数与最终日志SHA256见[validation.json](validation.json)。原日志留Git忽略的`.local/identity-validation/`，其中r1–r5与日志隔离before/after结果均保留。数据库、WAL/SHM、含token的邮件与challenge proof在独立私有临时目录，由测试夹具清理；不写公共证据或提交。

未执行真实PG16/Redis8集成与PG授权验证、完整安全/并发/性能、浏览器/正式视觉矩阵、真实供应商/邮件/反馈；仍留统一硬化。未推送、部署或创建特权入口。最终显式逐文件暂存，检查UTF-8/LF/末尾换行、无二进制/本机产物、有限敏感模式、旧迁移不变与暂存区diff。完整Secret扫描未执行，有限检查不替代安全门禁。
