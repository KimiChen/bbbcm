# 04 从空仓库开始的执行路线图

## 1. 总体方法

采用纵向切片和严格检查点。每个工作包交付可运行纵向切片；验证按 DEC-020 分层——开发期只过最小集，完整矩阵在统一硬化阶段集中执行。不能连续完成大量骨架后再补安全或证据。

执行主线：

```text
WP-MVP-A00 参考标的本地化（开发第一步，无依赖）
  -> 工程骨架与供应链
  -> 配置、迁移、ACL 与恢复底座
  -> 身份、邀请和账号生命周期
  -> 内容、搜索、媒体和互动
  -> 举报、治理、客服、后台和审计
  -> Outbox、故障、性能和发布门禁
  -> MVP 发布候选
```

## 2. 检查点通用输入

开始任一工作包前：

- 在 `99` 锁定精确需求 ID 和平台任务 ID。
- 阅读对应 `01` 条款及适用 `02/03/05`。
- 确认没有 Pending 决策会改变实现。
- 写明接口、schema/迁移、权限、事务、Outbox、Feature Flag 和测试；测试设计按 DEC-020 分层（开发期单元 + SQLite 冒烟，完整矩阵归入 `WP-MVP-E01`）。
- 后续阶段能力默认关闭且服务端不可达。

### 2.1 M0/MVP 主依赖矩阵

同一行依赖满足后可以与其他已就绪工作包并行；表中只列主依赖，逐需求的更细依赖在进入 `In progress` 前写入 `99`。

| 工作包 | 主依赖 |
|---|---|
| `WP-M0-01` | 无 |
| `WP-M0-02` | `WP-M0-01`、`DEC-019` |
| `WP-M0-03` | `WP-M0-01` |
| `WP-M0-04` | `WP-M0-01`、`WP-M0-03` |
| `WP-M0-05` | `WP-M0-01`、`WP-M0-03` |
| `WP-M0-06` | `WP-M0-01` 及对应产品、运营、法律决策 |
| `WP-MVP-A00` | 无 |
| `WP-MVP-A01` | `WP-M0-01`、`WP-M0-03` |
| `WP-MVP-A02` | `WP-MVP-A01` |
| `WP-MVP-A03` | `WP-MVP-A01`、`WP-MVP-A02`、`WP-M0-04` |
| `WP-MVP-A04` | `WP-MVP-A03` |
| `WP-MVP-A05` | `WP-MVP-A02`、`WP-MVP-A03`、`WP-MVP-A04` |
| `WP-MVP-A06` | `WP-MVP-A04`、`WP-MVP-A05` |
| `WP-MVP-B01` | `WP-MVP-A03`、`WP-MVP-A04`、`WP-MVP-A05`、`WP-MVP-A06` |
| `WP-MVP-B02` | `WP-MVP-A03`、`WP-MVP-A04`、`WP-MVP-A05`、`WP-MVP-A06` |
| `WP-MVP-B03` | `WP-MVP-A03`、`WP-MVP-A04`、`WP-MVP-A05` |
| `WP-MVP-B04` | `WP-MVP-A03`、`WP-MVP-A04`、`WP-MVP-A05`、`WP-MVP-A06` |
| `WP-MVP-B06` | `WP-MVP-A04`、`WP-MVP-A05`、`WP-MVP-B01`、`WP-MVP-B02` |
| `WP-MVP-C01` | `WP-MVP-A00`、`WP-MVP-A01`、`WP-MVP-A02` |
| `WP-MVP-C02` | `WP-MVP-A03`、`WP-MVP-A04`、`WP-MVP-B01`、`WP-MVP-B06` |
| `WP-MVP-B05` | `WP-MVP-B01`、`WP-MVP-B04`、`WP-MVP-C01`、`WP-MVP-C02` |
| `WP-MVP-C03` | `WP-MVP-A04`、`WP-MVP-A06`、`WP-MVP-B04`、`WP-MVP-C02` |
| `WP-MVP-C04` | `WP-MVP-A04`、`WP-MVP-B04`、`WP-MVP-B06`、`WP-MVP-C02` |
| `WP-MVP-C05` | `WP-MVP-A03`、`WP-MVP-A04`、`WP-MVP-A06`、`WP-MVP-B04`、`WP-MVP-C02` |
| `WP-MVP-D01` | `WP-MVP-A04`、`WP-MVP-B06`、`WP-MVP-C02`、`WP-MVP-C03` |
| `WP-MVP-D02` | `WP-MVP-B03`、`WP-MVP-D01` |
| `WP-MVP-D03` | `WP-MVP-A04`、`WP-MVP-B02`、`WP-MVP-B03`、`WP-MVP-B04`、`WP-MVP-D01`、`WP-MVP-D02` |
| `WP-MVP-D04` | `WP-MVP-A04`、`WP-MVP-A06`、`WP-MVP-B03`、`WP-MVP-D01`、`WP-MVP-D02`、`WP-MVP-D03` |
| `WP-MVP-E01` | 所有 A-D 工作包 |
| `WP-MVP-E02` | `WP-MVP-E01`、已完成的 A-D 性能关键路径 |
| `WP-MVP-E04` | 所有 MVP 用户界面、`WP-M0-02`（含 `DEC-019`）、`WP-MVP-A00` 静态夹具、`WP-MVP-C02` 确定性数据库夹具 |
| `WP-MVP-E05` | `WP-MVP-E02/E04` 及全部 M0/MVP 工作包 |

任何 Pending 决策只阻塞受影响行和 Production Approved；除非它会改变数据模型、安全边界或不可逆实现，否则不得阻塞其他已就绪工作包。具体地：

- `DEC-016`/`DEC-018` 只阻塞**生产 IAM 授权与角色种子**，不阻塞 `WP-M0-04` 的 schema、迁移、事务与 Outbox 设计。
- `DEC-019`（视觉基准，`10` 第 3 节第 5 项）只阻塞 `WP-MVP-E04` 与 Production Approved，不阻塞 `WP-MVP-A01` 及其下游。
- `WP-MVP-A00` 依赖为「无」指的是无**工作包**依赖；它需要 `bbbcm-web` 可运行，因此 `WP-MVP-A01` 的 workspace 骨架先行是必要顺序，不构成路线图冲突。
- `WP-MVP-B05` 的主依赖含 `C01`/`C02`，实际执行排在波次 C 末尾（`20` 第 3 节已注记）。
- `INVITE-004` 的管理侧 UI 由 `WP-MVP-D04`/`ADMIN-004` 承载，`WP-MVP-B01` 只交付邀请码模型与配额。

## 3. M0 工作包

`WP-M0-05` 已按 `DEC-036` 启用，承接容量与性能基线模型（见下）。

### `WP-M0-01` 计划与决策冻结

交付：316 个 ID、阶段映射、18 条 MVP 门槛、决策表和风险表。

退出：无未经登记的冲突；所有 Pending 有阻塞范围和未决定时行为。

### `WP-M0-02` 视觉基准批准

交付：`DEC-019` 批准记录与 `visual-baseline-v1` 冻结记录（在 `WP-MVP-A00` 产出上定版）。参考站采集、脱敏说明、目标品牌资产清单与 SHA-256、设计 token 与计算值清单、稳定夹具定义已前移至 `WP-MVP-A00`（`DEC-021`）；六视口截图矩阵、自动差异命令与阈值测试在统一硬化阶段交付（`DEC-020`）。

退出：`DEC-019` 已批准且基准在 `WP-MVP-A00` 冻结产出上定版可重复；属批准阶段工作，未批准则本工作包保持 Blocked。

### `WP-M0-03` 技术与供应链基线

交付：Rust toolchain、Cargo 依赖策略、PostgreSQL 16/Redis 8 镜像及 pg_jieba/pg_trgm 来源、版本、平台与摘要，pg_jieba 递归源码和固定编译依赖的 Dockerfile；**Node/npm/esbuild 版本及共用 `package-lock.json`**（`DEC-032`）、SBOM（含前端依赖）、许可证、漏洞与 Secret 的规则、工具版本和复现入口。当前配置为 `supply-chain/lock.json`、根目录 Cargo/npm 锁文件、`scripts/supply-chain*.py` 与 `supply-chain/README.md`，实际证据见 `99` 第 5.2 节。SBOM/Secret/许可证与漏洞扫描的集中执行属统一硬化阶段（`WP-MVP-E01`，DEC-020），本工作包锁定规则与版本。

退出：版本均锁定，可离线或受控重复构建；没有未处理 Critical/High 风险的判定属硬化/批准阶段。

### `WP-M0-05` 容量与性能基线模型

需求：平台任务，不直接代表产品需求完成。

交付：容量模型（1 万注册用户、300 峰值在线、100 万主题与回复）、200 read/s 与 20 write/s 的压测起点定义、关键路径的 p95 目标（公开读 400ms、写 800ms、SSR 首屏 800ms）、数据集生成方式与测量口径（地区、设备、缓存冷热、测量窗口）。本工作包只建**模型与口径**，实测基线属统一硬化阶段 `WP-MVP-E02`（`DEC-020`）。

退出：模型、口径与数据集生成方式经评审；具体 VPS 规格待 `DEC-004-B`，不填猜测默认值。

### `WP-M0-04` 数据与权限设计

交付：15 个逻辑 schema、角色/ACL、迁移设计、事务锁顺序、保留矩阵和 Outbox 设计。

退出：默认/非默认前缀方案经评审；`DEC-016/018` 未解决时生产 IAM 仍 Blocked。

### `WP-M0-06` 冷启动与合规

需求：`CAT-000/001`、`PRIV-000/005`、`TRADE-000/001`。

交付：人员、排班、种子内容、政策版本、允许/禁售边界和责任人清单。

退出：真实人员、内容和法律批准到位；仓库夹具不能代替。

## 4. MVP 平台工作包

### `WP-MVP-A00` 参考标的本地化

需求：平台任务，不直接代表产品需求完成。

重做的当前状态、负责人、交付边界与适用性设计统一登记在 `99` 第 5.1 节，本节只定义稳定工作包范围。本轮视觉冲突按 `DEC-043` 采用当前公开实测；原始参考截图按 `DEC-044` 留本机交用户后处理；资产按用户更新后的 `05` 与 `DEC-045` 如实登记许可并本机版本化，不部署、不推送。

交付：采集 `sb.sb` 参考页面的静态副本（HTML 结构、CSS、字体、图片、图标），按 `05` 本地化后落本地 `apps/web/static/` 与 `apps/web/templates/`；含文字资产替换为“草莓论坛”；以稳定静态夹具由 `bbbcm-web` 直接渲染，无数据库依赖；产出模板结构与 CSS 类名冻结清单，供 `WP-MVP-C01` 继承。禁止热链参考站任何资源。本轮采集限定与资产登记顺序按 `DEC-046` 执行，资产清单可在本机提交后补齐。

开发期验证：格式化、Clippy `-D warnings`、单元测试与本地可浏览冒烟。

硬化阶段（`WP-MVP-E04`）：像素级验收与视觉门禁；`DEC-019` 基准批准属 M0（`WP-M0-02`）。本地化本身不宣称视觉门禁通过。

### `WP-MVP-A01` Workspace 与工具链

需求：平台任务，不直接代表产品需求完成。

交付：Rust workspace、`bbbcm-web`、`bbbcm-worker`、`bbbcm-cli`、格式化、Clippy、测试、依赖审计、Secret 扫描和构建产物。

已落地（状态与证据见 `99`）：16 成员 workspace、三个可执行、`rustfmt.toml`、`rust-toolchain.toml`，以及 `scripts/dev-check.sh` 开发期最小集入口。工具链/锁文件和依赖审计、Secret 扫描规则/入口由 `WP-M0-03` 共用交付，不在 A01 建第二套。A01 待交付实际 esbuild 静态资产构建脚本——输入 `apps/web/static/{css,js,img,fonts}`，输出 `static/dist/` 内容哈希文件与 `manifest.json`，由 `apps/web/src/assets.rs` 读取，指纹路径响应 `Cache-Control: immutable, max-age=1y`，源路径不入生产；M0-03 锁定前置不等于 A01 完成。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：干净环境锁定构建、工具链版本、失败门禁、缓存不污染版本库。

### `WP-MVP-A02` 配置与 Feature Flag

交付：四环境配置 schema（含 `server.environment` 枚举与 `deny_unknown_fields`）、脱敏模板、Secret 注入接口、默认全关 flag、紧急关闭和只读模式，以及只读模式与紧急关闭的用户可见提示态（复用 `02` §2.2 的错误/空状态必填块，文案与 token 遵守 `05`）。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：非法/缺失配置拒绝启动；staging/production 拒绝 demo；所有 P1/P2 flag 关闭。

### `WP-MVP-A03` PostgreSQL 迁移与 ACL

交付：N 轨迁移集合（postgres/sqlite，mysql8 占位，编号对齐，DEC-022）、初始 schema、角色、ACL policy、迁移元数据和幂等校验、共享 SQL lint（`02` §6.1 语义差异禁用清单的强制手段）、`bbbcm-cli db seed` 开发/测试种子（四个首发版块骨架与规则占位版本；角色与权限种子受 `DEC-016/018`、`CR-013` 阻塞，未批准前不生成生产角色种子）。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：真实 PostgreSQL 默认/随机前缀、SQLite WAL/外键/单写者租约、未来对象 ACL 和前向修复。

### `WP-MVP-A04` 事务、幂等、Outbox 和审计

交付：事务模板、幂等状态机、Outbox lease/sink/DLQ 和追加操作日志。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：同/异载荷重放、未知提交、`40001/40P01`、多 Worker 和重复投递。

### `WP-MVP-A05` Redis 与可观测性

交付：`bcm:` namespace（可选 Redis）、Redis 限流与缓存（`features.redis = false` 时不提供限流，`DEC-024`）、`CacheStore`/`Wakeup`/`Lease`/`RateLimiter` 的最终异步签名冻结、结构化日志、指标、trace、健康检查（liveness/readiness/依赖降级三分）、依赖故障映射，以及 `/status` 公开状态页（服务状态、计划维护与事故公告，数据源为健康检查与人工公告）。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：真实 Redis 原子性、namespace 清理、Redis 消失后从 PostgreSQL 重建、日志敏感扫描。

### `WP-MVP-A06` 数据分析与外部适配器

需求：`DATA-001` 至 `DATA-006`、`INTEG-001/006`、`INTEG-005`（**仅代码面**：事务/营销邮件分类与邮件头、投递/延迟/退信/投诉率指标埋点与告警阈值；发送域 SPF/DKIM/DMARC 属运维面，见 `91` 第 4 节与 `10` 第 4 节，阻塞于 `DEC-011`）；`INTEG-002/003` 随 `C05`，`INTEG-004` 随 `A04`。

交付：版本化事件字典、去重与排除规则、脱敏聚合，以及邮件、对象存储、图片处理、恶意扫描、错误追踪和基础分析的可替换 adapter。未批准供应商时只提供本地测试实现和关闭配置，不填生产默认值。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：事件版本/时区/延迟、重复消费、排除账号、最少数据、日志泄露、adapter 合同和各 provider 故障语义。

## 5. MVP 身份工作包

### `WP-MVP-B01` 注册、邀请与验证

需求：`AUTH-001/002/003/008/010`、`INVITE-001` 至 `INVITE-006`。

交付：邀请配额、原子注册、同意版本、邮箱验证和统一枚举防护。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：默认/非默认 schema、并发邀请码单赢家、幂等、过期/撤销/冻结、验证前禁止发言。

### `WP-MVP-B02` 登录、恢复与认证风控

需求：`AUTH-004` 至 `AUTH-007`。

交付：登录/退出、找回/重置、密码策略、固定成本失败路径、四维限流和风险挑战 adapter。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：会话撤销、token 单次、并发找回、枚举/时序、Redis/provider 故障和 HTTP 错误态。

外部阻塞：SMTP、验证码和泄露密码 provider 未批准时只交付 adapter 与关闭状态。

### `WP-MVP-B03` MFA 与管理会话

需求：`AUTH-011` 至 `AUTH-013`、适用 `ADMIN-*`。

交付：TOTP、恢复码、MFA assertion、管理会话、重新认证、双人恢复、break-glass，以及首账号引导与 CLI 提权审计（`AUTH-014`）。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：跨用户、过期、重放、并发单赢家、旧 authz version、自批拒绝和告警接口。

### `WP-MVP-B04` 资料、偏好与生命周期

需求：MVP `USER-*`（`USER-003` 由 `WP-MVP-C03` 认领）、`PREF-*`、适用 `PRIV-*`。

交付：资料、头像、集合页、邮箱/用户名变更、设备会话、数据导出、注销冷静期和撤销。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：owner 边界、隐私三态、并发更新、旧会话撤销、导出授权和删除传播。

### `WP-MVP-B05` 新用户引导

需求：`AUTH-009`、适用 `CAT/TOPIC/MOD`。

交付：兴趣版块、规则阅读、精选主题、首次有效贡献和可恢复进度。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：隐藏内容拒绝、状态不能由客户端伪造、无 JavaScript 和移动键盘流程。

### `WP-MVP-B06` T0/T1 信任与基础风控

需求：`TRUST-001`、`RISK-001` 至 `RISK-008`。

交付：版本化 T0/T1 发布门槛快照、标准 `RiskEvent`、组合信号、渐进处置、人工复核和隐私最小化；T2/T3、自动升降级、积分和完整信誉体系保持关闭。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：规则版本与输入快照可解释，账号/IP/设备/网段不被单信号封禁，注册与高风险写在风控依赖失败时关闭，普通公开读取可降级。

## 6. MVP 内容工作包

### `WP-MVP-C01` 品牌外壳与公共读取

需求：`NAV-001` 至 `NAV-009`。

交付：首页四视图、版块/标签、主题行、右栏/抽屉、主题模式、签名分页和最近浏览隐私。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：SSR、无 JavaScript、六视口、明暗主题、溢出、稳定游标和公开权限。

### `WP-MVP-C02` 内容模型与发布

需求：MVP `CAT-*`、`TOPIC-*`。

交付：版块、标签、主题、回复、楼层、可见性、状态、修订、编辑/删除窗口和永久链接。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：原子发布、并发楼层、权限收紧、删除/恢复、移帖阻塞语义、XSS 和幂等。

另需产出 `tests/fixtures/` 的确定性数据库夹具生成命令（固定 ID、固定时间戳、固定排序），供 `05` 第 12 节与 `WP-MVP-E04` 使用。

### `WP-MVP-C03` Markdown、草稿和媒体

需求：MVP `EDIT-*`、`USER-003`。

交付：Markdown 白名单、预览、草稿、粘贴/拖放/选择、隔离扫描、转码、下载鉴权和清理。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：XSS、MIME/文件头/解码预算、恶意文件、对象存储故障、终态重放、孤儿清理和无 JS 上传。

### `WP-MVP-C04` 互动与通知

需求：`SOCIAL-001`~`SOCIAL-004`、`NOTIFY-*`（`SOCIAL-005` 由 `WP-MVP-D01` 认领）。

交付：收藏、正向反馈、关注、阅读位置、站内通知、通知偏好和安全邮件请求。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：唯一约束、计数一致性、拉黑过滤、幂等通知、退订不影响安全邮件和 provider 故障。

### `WP-MVP-C05` 搜索、SEO 与 Feed

需求：MVP `SEARCH-*`、`SEO-*`。

交付：pg_jieba/pg_trgm 搜索、过滤、高亮、canonical、OG/Twitter、JSON-LD、sitemap、robots、RSS/Atom。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：中文相关性、私密/待审/删除排除、签名游标、注入、百万重建、爬虫和敏感泄露扫描。

## 7. MVP 治理工作包

### `WP-MVP-D01` 举报、案件和证据

需求：`MOD-001`~`MOD-004`、`MOD-008`、`MOD-012`、`SOCIAL-005`（`MOD-010` 规则与政策版本化的归属待计划维护者按 `01` 原文确认，暂记本包）。

交付：举报入口、去重、证据快照、案件队列、分派、动作和状态版本。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：对象存在/可见性、跨类型 ID、证据 C2/C3、重复举报和并发处理。

### `WP-MVP-D02` 处罚与申诉

需求：`MOD-005`~`MOD-007`、`MOD-009`、`MOD-011`（处罚、限制、申诉与恢复）。

交付：类型化处罚、到期、撤销、独立申诉、通知和搜索/会话传播。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：职责分离、版本冲突、旧会话写阻断、撤销传播和操作日志。

### `WP-MVP-D03` 客服、恢复与隐私

需求：MVP `SUPPORT-*`、`PRIV-*`。

交付：公开/登录工单、账号恢复、紧急升级、数据请求、导出、删除和法律保留接口。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：目标绑定、联系方式隐私、双人恢复、下载授权、冷静期、墓碑和恢复后不重复副作用。

### `WP-MVP-D04` 管理后台

需求：MVP `ADMIN-*`。

交付：`03` 路由、闭合权限 registry、ABAC、审批、队列、批量、配置、作业、分析和审计。

开发期验证：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟（适用时）。

硬化阶段（`WP-MVP-E01`）：完整授权矩阵、直接 URL、ID 替换、旧版本、自批、批量上限、C3 零返回和浏览器 E2E。

## 8. MVP 发布工作包

`WP-MVP-E03` 为未使用编号，非删除或合并，不重编号（见 `99` 第 2 节）。

### `WP-MVP-E01` 统一硬化

交付：真实 PostgreSQL 双前缀集成、SQLite 全矩阵、Redis 合同、安全套件、并发测试、证据 manifest 补齐、SBOM/secret/许可证扫描（DEC-020 推迟项集中执行）。

本工作包一次性承接 22 个工作包推迟的验证，规模与其余任一工作包不在同一量级，必须分子段按序执行，每段独立留存证据、独立可重跑（重跑范围见 `90` 第 4.1 节）：

| 子段 | 内容 | 前置 |
|---|---|---|
| `E01-a` 环境与夹具 | 容器化 PostgreSQL 16 + Redis 8、`pg_jieba`/`pg_trgm` 构建镜像、双前缀（默认 `bcm_` 与随机非默认）数据库、确定性数据库夹具 | 全部 A–D |
| `E01-b` 数据层 | 迁移双前缀执行与幂等、N 轨编号对齐、ACL 未来对象探针、事务与重试、SQLite WAL/单写者/`BEGIN IMMEDIATE` | `E01-a` |
| `E01-c` 平台合同 | Outbox 多 worker 与租约、重放与 DLQ、sink 去重、Redis 原子性与清空重建、限流失败关闭 | `E01-b` |
| `E01-d` 领域集成与 E2E | 五条核心流程 E2E、双后端矩阵（PostgreSQL 全量／SQLite 核心流程 + 降级断言） | `E01-c` |
| `E01-e` 安全与并发 | XSS/CSRF/注入/越权/枚举/SSRF/上传绕过/重放/限速套件、关键路径 1,000 路并发 | `E01-d` |
| `E01-f` 供应链与证据 | SBOM、secret 与许可证扫描、证据 manifest 汇总与 SHA-256 | `E01-e` |

退出：各工作包推迟到本阶段的验证项全部通过并留存证据；相关需求与工作包方可凭证据标记 `Verified`。

### `WP-MVP-E02` 性能、并发与容量

交付：1 万用户、100 万主题/回复数据集；200 read/s、20 write/s 基线；关键 1,000 路并发测试。

退出：固定 p95、连接池、重建和资源曲线达标，报告可复核。

### `WP-MVP-E04` 浏览器与视觉

交付：六视口、明暗、键盘、无 JS、错误态、WCAG、资产和视觉差异报告。

退出：`05` 全部门槛通过，无未批准例外。

### `WP-MVP-E05` 全量回归与发布候选

交付：全量回归、冻结 commit、配置和 flag 快照。

退出：149 个 MVP 需求中非 `Blocked` 项全部至少 `Implemented`，`Blocked` 项逐条链接阻塞它的待决决策；18 条门槛通过；Repository Ready 成立。**`Verified` 不是发布候选的准入门槛**（`DEC-039`）：任何需求都可以以 `Implemented` 进入发布候选，`Verified` 只在统一硬化阶段凭证据授予。

## 9. 后续阶段

`DEC-014` 的量化阈值是**排期与启用**条件，不是编码开发的前置；未决定时按 `06` 的未决定时行为保持全部后续 flag 关闭且服务端路由不可达。P1-A、P1-B 可在 MVP 发布后并行开发，但必须独立门禁和开关。P2 依赖账本、风控、法律和活动完整性。P2/Later 与 Later 只有在 `DEC-014` 量化阈值达成后才能排期和启用。
