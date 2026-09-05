# 99 追踪、状态与证据索引

## 1. 初始状态

本计划假设没有任何源码、迁移、配置、测试、部署或历史证据。因此：

- 316 个需求初始状态全部为 `Not started`（初始基线 317；`DEC-031` 新增 `AUTH-014`，`DEC-034` 删除 `FEED-101`／`FEED-102`）。
- 所有工作包初始状态为 `Not started`。
- 没有 `Implemented`、`Verified`、`Release candidate` 或 `Released` 项。
- 既有源码或历史计划的实现和证据不得导入本计划状态。

开发开始前，本文件只维护工作包与门禁的汇总状态；逐需求的状态、责任人和证据在对应需求进入 `In progress` 时起登记于本文件，预定工作包与职责角色只是计划归属，不是实现、人员到岗或验收证据。

开发期需求与工作包状态最高登记 `Implemented`；`Verified` 及以上状态在统一硬化阶段凭证据授予（`DEC-020`）。

## 2. 基线计数

| 项目 | 数量 |
|---|---:|
| 唯一需求 | 316 |
| M0 | 6 |
| MVP | 149 |
| P1-A | 100 |
| P1-B | 18 |
| P2 | 28 |
| P2/Later | 14 |
| Later | 1 |
| 领域验收 | 57 |
| 非功能要求 | 53 |
| MVP 上线门槛 | 18 |
| 关键流程 | 6 |
| 数据不变量 | 8 |
| 关键索引/约束 | 11 |
| 全局 DoD | 11 |
| 工作包 | 57 |

工作包计数可复核：初始基线 54 = M0 5（`WP-M0-01`~`04`、`06`）+ MVP 24 + P1-A 9 + P1-B 4 + P2 6 + Later 6；现 57 = 54 + `WP-MVP-A00`（`DEC-021`）+ `WP-MVP-E01`（`DEC-020`）+ `WP-M0-05`（`DEC-036`，容量与性能基线模型）。`WP-MVP-E03` 仍为未使用编号，非删除或合并，不重编号。

### 2.1 P1 归一化表（`DEC-027`）

`01` 第 4 节有 74 条需求挂在未细分的「#### P1」小节下。按 `01` §0.2 的领域规则归一化后：P1-A = 显式 29 + 本表 71 = 100，P1-B = 显式 15 + 本表 3 = 18。新增或改动 P1 条目时必须同步本表。

| `01` 小节 | 归 P1-A | 归 P1-B |
|---|---|---|
| 4.2 注册、登录与新用户引导 | `AUTH-101`、`AUTH-102`、`AUTH-103`、`AUTH-104`、`AUTH-105` | — |
| 4.3 用户资料、关系与账号生命周期 | `USER-101`、`USER-102`、`USER-103`、`USER-104` | `USER-105` |
| 4.4 版块、标签与内容结构 | `CAT-101`、`CAT-102`、`CAT-103`、`CAT-104` | — |
| 4.5 主题、回复与阅读体验 | `TOPIC-101`、`TOPIC-102`、`TOPIC-103`、`TOPIC-104`、`TOPIC-105` | — |
| 4.6 Markdown 编辑器、草稿与附件 | `EDIT-102`、`EDIT-103`、`EDIT-104` | — |
| 4.7 收藏、反馈、打赏与内容关系 | `SOCIAL-101`、`SOCIAL-102`、`SOCIAL-103`、`SOCIAL-104`、`SOCIAL-105` | — |
| 4.9 搜索、SEO、RSS 与分享 | `SEARCH-101`、`SEARCH-102`、`SEARCH-103` | — |
| 4.10 邀请、激活与增长归因 | `INVITE-101`、`INVITE-102`、`INVITE-103`、`INVITE-104`、`INVITE-105` | — |
| 4.12 特殊主题：投票与悬赏 | `POLL-101`、`POLL-102`、`POLL-103`、`POLL-104`、`POLL-105`、`BOUNTY-101`、`BOUNTY-102`、`BOUNTY-103`、`BOUNTY-104`、`BOUNTY-105` | — |
| 4.15 举报、审核、处罚与申诉 | `MOD-101`、`MOD-102`、`MOD-103`、`MOD-104`、`MOD-105`、`MOD-106` | — |
| 4.16 管理后台与运营工具 | `ADMIN-101`、`ADMIN-102`、`ADMIN-105`、`ADMIN-106`、`ADMIN-107` | `ADMIN-103` |
| 4.17 隐私、个性化与订阅设置 | `PRIV-101`、`PREF-101`、`FEED-103`、`FEED-104` | — |
| 4.18 数据分析、增长与社区健康 | `DATA-101`、`DATA-102`、`DATA-103`、`DATA-105`、`DATA-106` | `DATA-104` |
| 4.22 风控与反作弊 | `RISK-101`、`RISK-102`、`RISK-103`、`RISK-104`、`RISK-105` | — |
| 4.23 客服工单与用户支持 | `SUPPORT-101`、`SUPPORT-102` | — |

### 2.2 需求编号说明

- 需求 ID 的数字段**不表示阶段**。阶段一律以 `01` 第 4 节的「#### 阶段」小节标题为准。`ADMIN-104` 属 P2、`TRUST-002`~`004` 与 `REP-001`~`004` 属 P1-A，均为例证。
- `CAT-002`~`CAT-009` 与 `EDIT-101` 是未分配的预留编号，非删除或合并。
- 不为编号连续而重编号；新增条目在各前缀现有最大号之后续接（`AUTH-014` 即按此规则新增）。

## 3. 状态登记规则

- 需求状态使用 [`README.md`](README.md) 的状态模型；工作包与门禁汇总状态见本文件第 4、5 节。
- 进入 `In progress` 前必须确认实际责任人，并补齐全部适用设计：依赖、交付物、接口、Schema/迁移、权限、Outbox、测试和 Feature Flag；不适用项写明原因。其中测试设计按 `DEC-020` 分层：开发期只设计单元测试与 SQLite 冒烟，完整验证矩阵设计归入硬化工作包。
- 开发期进入 `Implemented` 前需求整条交付并链接最小集（格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟、`git diff --check` 与暂存区复核）运行证据，开发期状态最高 `Implemented`；`Verified` 及以上在统一硬化阶段凭当前版本完整证据授予。

## 4. 决策索引

决策与冲突的当前事实统一见 [`06-decisions-and-risks.md`](06-decisions-and-risks.md)。已批准：`DEC-020` 验证分层、`DEC-021` 参考标的本地化先行（`WP-MVP-A00`）、`DEC-022` 三库可移植 SQL 子集与迁移 N 轨（目标版本及能力边界由 `DEC-033` 修订为 MySQL 8.0），以及 `DEC-030` 的全部七项产品参数（详见 `06`）。初始生产阻塞至少包括：

- `DEC-002/003-B` 流量、内容负责人和冷启动。
- `DEC-004-B/013` 生产规格、费用和扩容。
- `DEC-006-B/007-B/008-B/010-B` 法律、交易、材料和奖品。
- `DEC-011/012` 外部 provider、词典和责任人。
- `DEC-014` 后续阶段阈值。
- `DEC-016/017/018/019` SiteOwner、移帖、权限 registry 和视觉基准。

### 4.1 Repository Ready 阻塞（区别于生产阻塞）

下列待决决策直接定义某些需求的行为，未批准前对应需求记 `Blocked` 而非 `Implemented`；它们阻塞的是 Repository Ready，不只是 Production Approved：

| 待决 | 受阻需求 | 说明 |
|---|---|---|
| `DEC-017` 移帖跳转语义（`CR-012`） | `CAT-014` | 旧链接跳转的 ID 语义未定，功能整体不可实现 |
| `DEC-019` 视觉基准冻结 | `05` 第 15 节全部门槛 | 无批准基准则像素门禁无参照 |
| `DEC-011`／`CR-014` 泄露密码检查 | `AUTH-005` 的泄露检查子句、`AUTH-007` | provider 与 fail-open/closed 语义未定 |
| `DEC-011` 外部 provider | `EDIT-009`、`EDIT-013` | 对象存储与恶意扫描 provider 未选 |

判定规则见 [`README.md`](README.md) 第 6 节：外部 provider 依赖类需求若 adapter、故障语义、配置开关与本地测试实现均已交付且能力按已批准的未决行为保持关闭，可记 `Implemented`；行为完全由待决决策定义的需求记 `Blocked`。

## 5. 工作包状态

| 工作包范围 | 当前状态 | 证据与剩余工作 |
|---|---|---|
| `WP-M0-01` | Implemented（开发期，2026-09-05） | 执行责任：Codex；批准责任：项目所有者（用户）。已冻结并交叉复核 `01` 的 316 个唯一需求及阶段映射、18 条 MVP 门槛，`04` 的工作包与依赖，以及 `06` 的决策、待决事项、冲突和风险；所有当前 Pending 均有责任角色、阻塞范围和未决定时行为。开发期复核覆盖需求计数、阶段计数、过期口径扫描、`./scripts/dev-check.sh`、`git diff --check` 与暂存区复核；状态止于 `Implemented`，不据此授予 `Verified` 或任何发布门禁。 |
| `WP-M0-02` | Blocked | `DEC-019` 尚未批准，无法冻结 `visual-baseline-v1`；阻塞只落在 `WP-MVP-E04` 与 Production Approved，不阻塞 `WP-MVP-A00` 重做。 |
| `WP-M0-03` | Implemented（开发期，2026-09-05） | 技术与供应链基线已交付：保留 Rust/Cargo、共用 Node/npm/esbuild 锁文件、固定测试镜像与扩展输入、扫描配置及复现入口。负责人、依赖、交付物、测试设计与证据见第 5.2 节；沿用现有 Workspace 与 A00，未授予 Verified 或推进任何阶段/发布门禁。 |
| `WP-M0-04` | Implemented（开发期，2026-09-05） | 完成15逻辑schema/受信注册、数据库角色与ACL、迁移流程/元数据、固定锁序、幂等/Outbox/追加审计及16类保留矩阵派生设计；独立复核与最小集通过，责任、依赖、交付和证据见第5.4节。A04引擎当前交付见第5.5节，法律/IAM待决保持后续边界，不推进产品需求或正式门禁。 |
| `WP-M0-05` | Implemented（开发期，2026-09-05） | 模型、负载档案、确定性生成方式、单位与分位数口径及独立复核完成；10项模型测试，未灌百万数据或实测，见第5.5节。 |
| `WP-M0-06` | Not started | 冷启动与合规尚未交付。 |
| `WP-MVP-A00` | Implemented（开发期重做，2026-09-05） | 14个隔离SSR夹具、30项本地源资产、模板结构/CSS类名冻结、17张本机原始参考截图和开发期检查已交付，详见第5.1节。按用户修改后的 `05` 与 `DEC-045` 如实登记未知许可并本机版本化；E04/DEC-019仍未执行，不授予 `Verified`。代码版本为本条记录所在实现提交。 |
| `WP-MVP-A01` | Implemented（开发期，2026-09-05） | 沿用16成员Workspace、三个可执行与七步dev-check；复用 M0-03 锁定的 Node/npm/esbuild 和根 npm 锁文件，完成真实资产构建、30项 SHA-256 产物与 manifest、Rust 加载校验/SSR helper、immutable 指纹路由、生产隔离及操作手册。原A00冻结未改，模板仅11处资源URL接线。5项构建测试、15项web测试、四环境492次HTTP/84个SSR变体/8次启动拒绝、最小集72个单元测试和51条HTTP冒烟、diff与暂存区逐文件复核通过；SQLite迁移明确跳过（A03）。本轮开发交付无剩余缺口，代码版本为本条证据所在的 `新增：完成 WP-MVP-A01 静态资产构建与指纹服务` 提交；完整说明、依赖、测试与证据见第5.3节。E01/E04完整矩阵和阶段/发布门禁仍未通过。 |
| `WP-MVP-A02` | In progress（开发期，2026-09-05） | 本轮配置前置完成：三程序共用安全默认/主文件/local/环境/Secret分层，四环境模板、严格解析、连接/前缀/Redis/Worker校验、Secret脱敏、PostgreSQL启动检查与手册；配置13项及CLI/连接回归随最小集通过。本轮另完成版本化Flag存储、一致只读加载与实际CLI list（第5.5节）；整包剩余受控flag set与/manage/config、审计接线/审批、只读/紧急关闭及提示态，依赖后续授权/A04/D04；14个MVP Flag仍默认全关。配置前置见第5.4节。 |
| `WP-MVP-A03` | Implemented（开发期，2026-09-05） | 已修复有效轨为空豁免；交付PG/SQLite真实迁移、15逻辑schema映射与3张必要初始表、专用单连接/前缀锁/元数据/漂移拒绝/失败回滚/FK恢复、受信表注册、PG组角色bootstrap与显式/默认ACL、实际migrate/status/seed、4版块规则占位和共享SQL lint。迁移20项及种子/ACL/CLI回归、真实SQLite与HTTP冒烟通过；角色与IAM区分，mysql8只占位，真实PG ACL/并发等留硬化。版本、设计和证据见第5.4节。 |
| `WP-MVP-A04` | Implemented（开发期，2026-09-05） | 事务/幂等/未知提交、Outbox调度/受控重放、追加审计/敏感读取、双宿主恢复与Webhook版本模型完成；独立复核及真实SQLite最小集通过，见第5.5节。无真实provider或授权路由，不授Verified。 |
| `WP-MVP-A05` | Implemented（开发期） | Redis异步合同、故障/轮询、有限观测、真实健康与C0状态公告；开发期验证和独立复核通过，代码/测试/证据见第5.6节。依赖按DEC-047细化，真实服务与完整门禁留硬化。 |
| `WP-MVP-A06` | In progress | 分析持久流水线与本地适配器/邮件代码面已交付；领域生产者、真实指标与看板仍缺，第5.6节逐需求裁决，不批量标完成。 |
| `WP-MVP-B01..B06` | Not started | - |
| `WP-MVP-C01..C05` | Not started | - |
| `WP-MVP-D01..D04` | Not started | - |
| `WP-MVP-E01/E02/E04/E05` | Not started | - |
| `WP-P1A-01..09` | Not started / flag closed | - |
| `WP-P1B-*` | Not started / flag closed | - |
| `WP-P2-*` | Not started / flag closed | - |
| `WP-LATER-*` | Not started / not committed / flag closed | - |

### 5.1 `WP-MVP-A00` 重做交付与硬化边界（2026-09-05）

| 设计项 | 登记内容 |
|---|---|
| 状态与责任 | 当前为 `Implemented`（开发期，代码及证据版本为本条记录所在提交）：2026-09-05 用户明确授权本轮从零重做、开发期验证及提交，不部署、不推送。负责人及最终批准人为项目所有者（用户）；Codex 负责实现、验证与提交，不恢复 `DEC-041` 已移除的实现或仓库外归档。 |
| 需求与依赖 | 平台任务，不直接代表任何产品需求完成。无工作包依赖；现有可运行的 `bbbcm-web` 骨架只是执行前提。`DEC-019` 只阻塞视觉基准批准、`WP-MVP-E04` 与 Production Approved，不阻塞本工作包实施。模板按 Askama 0.16 与 `02` 第 2.2 节执行。 |
| 交付物 | 新编写的 `apps/web/templates/`、`static/` 与 `src/{contract,fixtures,assets}.rs`；Askama 0.16.1。`docs/visual/a00/fixtures.json` 固定12条合成主题、4个合成果农、固定计数/时钟和顺序；`structure-freeze.json` 是开发冻结模板标签/CSS类名/源哈希清单；`assets.json` 登记30项源资产的完整元数据与许可；`reference/` 和 `implementation/` 分别保存当前公开测量与合成实现测量，旧称词表及结果见 `old-names.json`、`checks.json`、`asset-text-scan.json`、`ocr-summary.json`。全宽版块菜单与两行七控件页脚补测见 `supplemental-forum-menu.json`；搜索按钮与列表图标当前实测见 `search-control-observation.json`；品牌章/favicon/应用图标及OG生成记录见 `brand-localization.json`、`og-localization.json`；共享邮件页头明确为项目品牌组合，不冒称参考邮件已观察。未知许可按DEC-045完整登记，不冒充权利方许可。所有导航只切换本地夹具。没有读取旧实验或归档。 |
| 接口、数据与迁移 | 实际页面为 `/__fixtures/a00/{page}`：`home`、`new`、`pinned`、`featured`、`forum`、`topic`、`login`、`register`、`reset`、`policy`、`search`、`empty`、`error`、`profile`；支持 `?theme=light\|dark\|system`，默认system，非法主题400、未知页404。`search`复现当前公开HTTP200/页面显示200的错误外壳，不虚构成功搜索结果。`profile`依据匿名公开个人主页实测，身份/头像/统计/进度全部固定合成；390px用户侧栏保留在列表下方。源资产为 `/__fixtures/a00/assets/{name}` 下的 `site.css`、`pages.css`、`profile.css`、`fonts.css`、`brand-mark.svg`、`theme-sun.svg`、`theme-moon.svg`、`ui-menu.svg`、`ui-chevron-up.svg`、`ui-search.svg`、`ui-eye.svg`、`ui-chat.svg`、`ui-user.svg`、`ui-document.svg`、`ui-forum.svg`、`oauth-google.svg`、`oauth-github.svg`、`oauth-x.svg`、`brand-symbol.svg`、`favicon-16.png`、`favicon-32.png`、`favicon.ico`、`apple-touch-icon.png`、`og-image.svg`、`og-image.png`、`email-header.svg` 与四个 `space-mono-{latin,latin-ext}-{400,700}-normal.woff2`；明确允许列表、no-store、nosniff，未知资产404。清单已同步 `scripts/smoke-routes.txt`。无业务写API、数据库读写、Schema或迁移；`/`仍为原骨架首页。 |
| 权限 | 不适用业务权限。账号控件只读、无提交表单，POST405。`DEC-044`允许原始参考截图本地保留，由用户后处理。采集限定已按 `DEC-046` 移除；同一决定取消资产逐项登记完成后才能本机提交的前置顺序，资产清单仍为交付项。 |
| Outbox | 不适用。本工作包没有业务事务、状态写入、领域事件或外部副作用；不得为静态夹具虚构 Outbox 事件。 |
| 测试设计 | 开发期运行格式化、Clippy `-D warnings`、单元测试、适用的 SQLite/HTTP 冒烟、`git diff --check` 与暂存区复核；另做 Askama 编译期模板检查、本地路由状态检查、夹具确定性、无热链、敏感内容/旧称扫描、资产哈希与清单完整性、实际许可状态/DEC-045本机版本化边界检查。六视口乘双主题、WCAG、几何和像素差异属于统一硬化阶段 `WP-MVP-E04`，本工作包不得据开发期结果宣称视觉门禁通过。 |
| Feature Flag | 不适用新增或启用业务 Feature Flag。本工作包不得改变现有 flag 默认全关状态，不开放 P1/P2 路由；静态夹具入口只允许在 development/test 环境可达，staging/production 必须失败关闭。 |
| 本轮批准与截图 | `DEC-043`：视觉规则冲突以当前公开实测值为准，已回写 `05`。`DEC-044`：已移除本轮15张脱敏参考PNG，并重新采集15张未遮罩截图到 `docs/evidence/local/a00-redo/reference/`（Git忽略），另补采2张匿名公开个人主页原图于同级 `profile/`，入口与哈希见 `profile-observation.json`，合计17张均由用户负责后处理。公开 `reference/manifest.json`只保存安全结构/计算值、本地截图路径与哈希，不提交原图；普通开发观察不是批准基准。`DEC-045`按用户修改后的文档允许本轮未知许可素材本机版本化；用户随后替换本任务goal并明确整体删除原执行要求第8项，不再作为要求或阻塞依据。资产哈希、清单和许可证检查仍按最新goal如实记录，未知许可为unconfirmed/pending，不部署、不推送。 |
| 开发期证据 | `docs/visual/a00/validation.md` 登记最终命令与结果；Askama编译、65个workspace单元测试（web8个）、Clippy、48条HTTP/SQLite配置冒烟、36文件/42种SSR字节确定性/无热链/旧称/敏感值扫描、30项资产完整性与实际响应哈希、37文件名称/字体/SVG/PNG/ICO元数据扫描、15张图的OCR与人工检查，以及diff检查通过。SQLite迁移执行器未接入，明确跳过，不记作迁移通过。暂存区逐文件复核后本机提交；不push、不部署。 |
| 剩余与硬化边界 | 本轮开发交付已完成。目标品牌图形保留当前观察的章形/颜色/尺寸并替换文字；已许可字体与Line MD/Feather继续保留许可，其他公开UI/OAuth/品牌图形按DEC-045记录unconfirmed/pending，不宣称第三方权利已解决。参考邮件与成功搜索结果未公开观察，不虚构私有布局或业务。移动抽屉背景Tab焦点、完整键盘/Escape/焦点恢复、SVG动画相位、六视口像素/几何/WCAG及DEC-019批准留 `WP-MVP-E04`；开发冻结不是 `visual-baseline-v1`，不标 `Verified`。 |

### 5.2 `WP-M0-03` 技术与供应链基线（2026-09-05）

| 设计项 | 登记内容 |
|---|---|
| 状态与责任 | 实施前登记为 `In progress`；本次开发期交付与检查完成后更新为 `Implemented`，代码/配置版本为证据记录所在的 `新增：完成 WP-M0-03 技术与供应链基线` 提交。项目负责人及最终批准人：项目所有者（用户）；Codex 负责本轮实现、验证和本机提交。用户明确授权完成本包，不部署、不推送。 |
| 需求与依赖 | 平台任务 `WP-M0-03`，对应 `01` §7.1、§8.1、阶段0技术锁定条款，不直接代表任何产品需求完成。主依赖 `WP-M0-01` 已 `Implemented`；沿用已交付 A00 和 Workspace。应用 `02` §2.3/2.4、§4/6/12/13 及 `DEC-020/032/039/045`。生产 provider、词典责任人和正式视觉基准待决不阻塞本机工具链与测试镜像锁定。 |
| 交付物 | 保留可用 Rust/Cargo 与 Cargo.lock，登记来源及更新策略；共用 Node/npm/esbuild 版本、package-lock.json 和受控安装入口；官方核实的 PostgreSQL 16、Redis 8、pg_jieba（含子模块）/pg_trgm 来源、版本、平台与摘要，以及扩展构建 Dockerfile；SBOM（含 npm）、许可证、漏洞、Secret 扫描的锁定工具、配置与复现入口；操作说明和开发期证据。 |
| 接口与构建边界 | 接口仅为本机工具链安装、锁定一致性检查、镜像构建/测试环境准备和按需扫描命令。M0-03 锁定构建前置；A01 共用这些工具链和锁文件，继续交付实际 esbuild 管线、static/dist 内容哈希产物、manifest helper 和 immutable 路由。本轮不修改 A00 冻结模板、CSS、截图或品牌资产。 |
| 数据、权限与 Outbox | 均不适用：无业务数据库、Schema、迁移、业务 API、权限或业务事务写入，不定义 Outbox 事件。镜像说明仅用于本机测试，不填生产连接值或部署配置。 |
| 测试设计 | 开发期检查锁定版本/来源/摘要格式与交叉一致性、官方来源核实、受控 npm ci 与 esbuild 可执行版本、扫描工具版本/配置及失败行为的轻量自检；运行 ./scripts/dev-check.sh、git diff --check 和暂存区逐文件复核。Docker 缺失时如实登记未运行镜像构建/容器测试。SBOM、许可证/漏洞/Secret 正式全量扫描、Critical/High 清零及真实 PostgreSQL/Redis/E2E 完整证据留 WP-MVP-E01；不向每次 dev-check 增加完整构建或扫描。 |
| Feature Flag 与资产许可 | 不新增、不启用业务 Feature Flag，现有默认全关不变。A00 资产登记与本机版本化沿用 DEC-045，未知许可维持 unconfirmed/pending；不恢复已取消的入库前置或把未知权利改标为已授权。 |
| 当前交付与证据 | 负责人/依赖/测试设计在实施前登记，并随用户并行提交 `6a93847` 保留。Rust/Cargo 1.97.0 与 Cargo.lock 未升级；Node 24.20.0/npm 11.19.0/esbuild 0.28.2 共用锁定；PostgreSQL 16.15、Redis 8.2.9 双平台 manifest/config 摘要、pg_jieba 1.1.1 三层源码与 pg_trgm 1.6、58 个 APK 的来源/摘要/包内元数据已核实，配置在 `supply-chain/lock.json`。入口为 `scripts/supply-chain.py`、`supply-chain-scan.py` 与 `test_supply_chain.py`，工具版本为 Syft 1.51.1、Trivy 0.74.0、Gitleaks 8.30.1。最新开发期证据见 [`summary.md`](../docs/evidence/m0/wp-m0-03/dev-20260905/summary.md) 及同级 `upstream.json`、`validation.json`。受控 npm ci 和离线重装、两个离线构建上下文、10 项合成夹具测试、./scripts/dev-check.sh（65 个单元测试、48 条 HTTP 冒烟）、git diff --check 和暂存区逐文件复核均通过；暂存区 Gitleaks 无发现，SQLite 迁移明确跳过。保留初次失败和修正记录，未以开发期结果替代 E01 全量验收。 |
| 剩余与硬化边界 | M0-03 不实现 A01 的资产构建管线、dist、manifest helper 或 immutable 路由。本机无 Docker，未运行扩展镜像构建、真实 PostgreSQL/Redis；派生镜像只锁输入及配方，没有虚构产物 digest。跨宿主重构建、实际镜像/完整候选 SBOM、许可证/漏洞/Secret 全量扫描、Critical/High 清零和正式证据由 E01 执行；E04/DEC-019 与各生产待决保持原范围。A00 未修改，DEC-045/046 的资产登记与本机版本化边界保持，不新增业务 Feature Flag。 |

### 5.3 `WP-MVP-A01` 静态资产构建与指纹服务（2026-09-05）

| 设计项 | 登记内容 |
|---|---|
| 状态与责任 | 实施前已登记为 `In progress` 并补齐设计；本轮全部开发期交付、针对性测试和最小集完成，当前为 `Implemented`。项目负责人及最终批准人：项目所有者（用户）；Codex 负责实现、验证和本机新建提交。代码版本为本条证据所在的 `新增：完成 WP-MVP-A01 静态资产构建与指纹服务` 提交。仅本机开发，不部署、不推送；本轮开始时已有的 `g.md` 改动保持未暂存。 |
| 需求与依赖 | 平台任务 `WP-MVP-A01`，对应 `01` §7.1、§7.5 内容哈希资源一年缓存、§8.1 与 `02` §2.2/2.3/2.4；不代表产品需求或 `ADMIN-005` 完成。主依赖 `WP-M0-01`、`WP-M0-03` 已 Implemented，复用现有 Workspace 和 A00。遵守 DEC-020/032/035/039/045/046；DEC-019 及生产 provider 待决不改变本包实现范围。 |
| 交付物 | 沿用三个可执行、16 成员 Workspace、七步开发期最小集及 M0-03 共用 Node 24.20.0/npm 11.19.0/esbuild 0.28.2、根 package-lock.json 与受控安装入口。`scripts/assets.py`、`build-assets.mjs` 编译 CSS/JS/TS 并指纹化图片/字体，生成30项 SHA-256 产物与 manifest；`assets.rs`、`fixture_assets.rs`、AppState/main/router 提供启动校验、SSR helper、指纹路由及原源资产环境隔离。模板只11处URL改为helper，原冻结记录不改。`test-assets.mjs`、`assets_tests.rs`、`test-assets-http.py` 和 `asset_freeze.py` 提供针对性验证；操作说明见 `docs/development/assets.md`。npm锁文件与版本未动，Cargo.lock仅给web链接已有 serde_json/sha2/thiserror，不升级registry包；dist、缓存与临时产物保持忽略。 |
| 接口与失败边界 | 独立按需构建/测试/清理命令；`/assets/{*path}` 仅返回经 manifest 校验的指纹资源，成功响应一年 immutable 和 nosniff，未知资源404、非法路径明确拒绝。启动加载后保留不可变内存快照，不按请求拼文件路径；staging/production 缺失或损坏 manifest 拒绝启动，源资产与 A00 夹具不可达。仅 development/test 在默认 dist 尚未生成时允许明确提示的 A00 源模式，使干净 dev-check 不依赖完整资产构建；显式指定目录或已有损坏 manifest 不回退。 |
| 模板与品牌覆盖 | A00 现有模板只允许资源 URL/helper 接线，保持标签顺序、CSS 类名与全部源资产字节；保留原 structure-freeze.json，不重新采集、截图、制作资产或重置冻结。DEC-035 只提供受限品牌覆盖解析契约及空覆盖装配，未来上传、对象存储、尺寸/格式校验、审批/审计与生效版本由 ADMIN-005 所属包交付，本轮不开放写入口。 |
| 数据、迁移、权限、事务与 Outbox | 全部不适用：无业务数据读写、Schema、迁移、权限或业务状态事务，不定义 Outbox 事件；资源允许列表和环境隔离是静态服务边界，不新增业务授权系统。 |
| 测试设计 | 按需运行实际 esbuild 构建验证：内容哈希、重复构建一致性、清单覆盖/文件摘要、CSS 字体及图片引用、无热链、JS/TS 合成夹具和失败保留旧产物；Rust 单元及路由测试覆盖 manifest 缺失/损坏/不完整、helper 错误与品牌覆盖优先级、SSR 实际指纹 URL、GET/HEAD 缓存头、404、非法路径、任意文件/源资产/夹具生产隔离。独立核对冻结模板只发生 URL 替换。随后 ./scripts/dev-check.sh、git diff --check、暂存区逐文件复核。SQLite 迁移未接入如实跳过；正式全量扫描、跨平台、安全与视觉完整矩阵留 E01/E04，不向每次 dev-check 无条件加入资产构建。 |
| Feature Flag 与状态边界 | 不新增、不启用任何业务 Feature Flag。原资产未知许可继续如实登记，DEC-045/046 已取消的本机版本化前置不恢复。只同步 `04`、`20`、`99` 实际交付与证据；不标记 Verified，不宣布阶段或发布门禁通过。 |
| 当前证据与缺口 | 实施前已核对 AGENTS、计划入口及01/02/04/05/06/10/20/90/99。最新证据见 [summary.md](../docs/evidence/mvp/wp-mvp-a01/dev-20260905/summary.md) 及同级 validation.json、http.json、freeze.json 和命令/日志。真实构建及5项构建测试、15项web测试、492次四环境HTTP/84个SSR变体/8次启动拒绝、M0-03基线一致性、无dist下 ./scripts/dev-check.sh（72个单元测试、51条HTTP冒烟）、git diff --check 与36个暂存文件逐文件复核通过；SQLite迁移执行器未接入，明确跳过。原A00全部源资产/冻结保持，实际SSR反向还原URL后与原42种响应字节一致。初次Python前置、占位文件、macOS路径和证据文件末尾空行失败均保留修正记录；本轮开发期交付无剩余缺口。真实依赖/全量扫描/跨平台/安全完整矩阵留E01，视觉与DEC-019留E04，ADMIN-005业务写入与对象存储未提前实施。 |

### 5.4 数据库基础开发登记（2026-09-05）

本轮由用户担任项目负责人和最终批准人，Codex 负责设计细化、实现、独立复核、开发期验证和本机新建中文提交；仅本机开发，不部署、不推送。工作包按 M0-04 设计 → A02 配置前置 → A03 迁移与 CLI 顺序实施。下表保留实施前范围与状态条件；本轮结果与证据补充于表后。沿用 M0-01、M0-03、A00、A01、现有 Workspace/锁文件与资产隔离；g.md 与 AGENTS.md 继续本机忽略。

| 设计项 | WP-M0-04 | WP-MVP-A02（本轮部分） | WP-MVP-A03 |
|---|---|---|---|
| 范围与原始条款 | 平台任务，01 §6、§7.2–7.4、§8.2；关联 PRIV-000/005、ADMIN-012/013 的设计边界，不推进这些产品需求。 | 平台任务，01 §7.1/7.2/8.1，02 §4/5/6.4；仅配置、Secrets、启动校验，不代表 ADMIN-005/011 完成。 | 平台任务，01 §6/7.2/7.3，02 §6–8/19；种子仅引用 CAT-000/001 的四版块骨架，不代表产品/冷启动完成。 |
| 依赖与责任 | M0-01、M0-03 已 Implemented；Codex 细化与独立复核，用户最终批准。DEC-016/018、CR-013 仅限制 IAM registry 和生产授权。 | A01 已 Implemented，复用 M0-04 设计；Codex 实现/测试。Flag 整包后续依赖 A04 审计/事务与 D04 审批。 | A01、M0-04 设计复核与 A02 配置前置；依用户本轮指示，不以 A02 整包状态阻塞迁移；Codex 实现/测试/复核。 |
| 交付物 | 可复核的数据设计：15 schema、受信注册、ACL、迁移元数据/流程、锁顺序、幂等、Outbox、追加审计和保留矩阵；设计文档是 01/02/03 的派生说明，不建立并行事实源。 | 共用分层加载、四环境脱敏模板、local 覆盖、环境变量/Secret 注入、严格解析和启动校验、三程序接线与手册。 | PostgreSQL/SQLite 真实对齐迁移、初始结构、数据库角色与显式 ACL、专用单连接 runner/status、开发种子、共享 SQL lint、dev-check 真实 SQLite 接线和手册。 |
| 接口与数据 | 文档化设计，不交付 A04 空接口；实际 A03 表精确登记到受信表注册。 | Config 共用入口保留 BBBCM_CONFIG；安全默认 < 配置文件 < local 覆盖 < 环境变量 < Secret 注入；不建立配置数据库。 | bbbcm-cli db migrate/status/seed；mysql8 预留拒绝执行；元数据编号、名称、内容哈希、执行状态与时间；其他业务表随所属包落地。 |
| 权限 | 区分 PG 数据库角色与业务 IAM；未批准保留期/法律依据明确待定，不自行批准。 | Secret 不入配置文件/Debug/日志/错误；provider 缺配置保持关闭；production 固定 bcm_，staging/production 拒绝不安全配置。 | migrator 拥有应用对象；app_rw/worker_rw/report_ro NOINHERIT、无 schema CREATE、未来对象默认无授权；审计历史无更新删除。SQLite 不模拟 PG ACL；seed 限 development/test，无账号或 IAM 种子。 |
| 事务与 Outbox | 定义固定跨领域锁序、同事务状态/版本/幂等/Outbox/审计、未知提交与追加审计边界；引擎归 A04。 | 不适用事务/Outbox：只加载本进程配置，无业务状态或外部副作用；Flag 持久化/审计不在本轮。 | 每迁移专用连接事务，PG 含前缀 advisory xact lock，SQLite BEGIN IMMEDIATE；重建迁移事务外关闭 FK、提交前检查、退出恢复；种子同事务幂等。初始化无业务事件，不伪造 Outbox。 |
| 开发期测试 | 默认/非默认映射、注册和 ACL 对照设计复核；保留矩阵待决范围复核。 | 分层优先级、未知键/类型/环境、缺失/非法参数、Secret 脱敏、四环境、Worker 模式；保持 A00/A01 隔离。 | 临时 SQLite：对齐/空轨/编号/内容漂移/元数据/重复/失败回滚/FK恢复、双前缀/未注册拒绝、实际 CLI/种子重放/生产拒绝、SQL lint 合法/非法例；./scripts/dev-check.sh、diff/暂存逐文件检查。 |
| Flag 与状态条件 | 无新增 Flag；设计交付及所需复核完成才可 Implemented，未决生产法律/IAM 不冒称获批。 | 默认全关，整包保持 In progress；待交付 Flag 存储、flag set 与 /manage/config 共用受控写/审计、只读/紧急关闭与提示态及审批。 | 开发交付与最小集全部完成才可 Implemented；不实现 A04–A06、身份/内容/后台业务，不启用业务 Flag。真实 PG/Redis、安全/并发/跨平台/性能/视觉留 E01/E02/E04，不标 Verified 或通过阶段门禁。 |


#### 本轮实际交付与开发期证据

- 实施前已完整核对 AGENTS、计划入口、01/02/03/04/06/10/20/90/99；未改提示页面、模板或品牌资产，无需新增05视觉交付。设计为 [数据库基础派生设计](../docs/design-notes/2026-09-database-foundation.md)，操作入口为 [本机数据库手册](../docs/development/database.md)。不改变01/02/03/05/06事实源或批准任何Pending；04只按本轮用户明确授权拆解A02配置前置依赖。
- M0-04 的15 schema、3初始表、ACL、迁移、锁总序、幂等/未知提交、Outbox/追加审计、16类保留矩阵已由实现代理和独立代理对照事实源复核；本轮设计交付无剩余缺口，标记 Implemented。A04事务/Outbox/审计引擎与保留政策批准仍是后续事项，PRIV-000/005、ADMIN-012/013、CAT-000/001等产品需求状态不变。
- A02配置前置完成并接到三个可执行。主文件缺失、逐层未知键/类型、非法环境/URL/前缀、Secret入文件、生产TLS降级、隐式PG环境/.pgpass旁路均拒绝；未批准provider没有新增配置或功能。Redis仅配置校验，连接运行时归A05。A02整包仍 In progress；待Flag数据库存储、两受控写入口、A04事务/审计、D04审批、只读/紧急关闭和05提示态完成后再评估。
- A03初始应用表仅schema_migrations、forums、rule_versions。PG迁移创建15 schema、显式表/序列ACL与全局及逐schema默认撤权；bootstrap输出NOLOGIN数据库组脚本，真实登录/成员配置按手册由数据库管理员执行。SQLite不伪造角色；种子只development/test且保留已有编辑，未创建账号/IAM/生产规则。本轮开发期交付与所需设计复核完成，标记 Implemented。
- 开发期最小集 `./scripts/dev-check.sh` 一次完整执行通过：rustfmt、Clippy `-D warnings`、99个单元测试及4个真实CLI集成测试（含配置13、迁移20、连接4、种子2、SQL lint8、ACL1、注册3和Web15项）、真实临时SQLite迁移、51条HTTP冒烟、diff检查和Handler静态守卫。资产冻结复核通过，A00/A01模板及源资产无改动；没有独立构建或完整资产构建。dev-check的暂存步骤运行时尚无暂存文件，最终逐文件复核单独记录，不能以空暂存冒称完成审查。
- 当前证据入口为 [summary.md](../docs/evidence/mvp/database-foundation/dev-20260905/summary.md)、同级dev-check.txt、asset-freeze.json和validation.json，包含代码复核、早期失败/修正及最终暂存记录。代码/配置版本为这些记录所在的 `新增：完成数据库基础设计与配置迁移底座` 提交；仅新建本机提交，不amend、不push、不部署。AGENTS.md、g.md、local配置、Secrets与临时数据库保持忽略。
- 未执行真实PostgreSQL/Redis全矩阵、真实数据库角色/未来ACL探针、并发/故障注入、安全全量扫描、跨平台、性能或视觉完整验证，留E01/E02/E04；未改变A04–A06及身份/内容/管理后台实现状态，没有Flag启用、Verified或阶段/发布门禁。

### 5.5 本轮基础引擎实施登记（2026-09-05）

本轮由项目所有者（用户）任负责人和最终批准人，Codex 承担实现、独立复核、开发期验证与本机提交。执行顺序为 M0-05 独立并行、A04 → A02 存储读取。沿用第5.4节数据库基础设计与现有 Workspace，不改已应用迁移、不另建计划事实源；仅本机，不推送或部署。

| 工作包 / 状态条件 | 实施前范围、依赖与交付设计 |
|---|---|
| `WP-M0-05` / Implemented（开发期） | 平台任务，依据01 §7.5/8.3/上线容量门槛、02 §17、DEC-020/036。交付容量派生模型、负载档案、确定性生成方式和口径检查。目标为1万用户、300在线、100万主题与回复、200 read/s及20 write/s，p95公开读400ms/写800ms/SSR800ms；地区、设备、冷热缓存、窗口、单位和指标算法明确。接口供A05观测、E02数据装载和测量接入；无运行时API、迁移、权限、事务或Outbox写入。模型假设与目标分列，生产规格/配额/保留期继续Pending；仅小规模合成验证，不灌百万数据、不压测。完整交付并独立复核后才可Implemented，不宣称性能达标。 |
| `WP-MVP-A04` / Implemented（开发期） | 平台任务；精确对应01 §7.3/7.4/8.1、`INTEG-004` 的Outbox及版本化Webhook预留、`ADMIN-012/013` 的本地审计合同部分。依赖M0-04/A03和A02既有配置；交付共享合同、双后端事务/幂等存储、Outbox/sink/lease/DLQ调度、两种Worker宿主、追加审计与恢复手册。platform放共享合同，platform-db放事务和存储适配，platform-outbox/platform-audit经共享合同注入，由apps组合，不增加横向依赖。仅新增同编号PG/SQLite引擎迁移、受信表注册和显式ACL；mysql8占位，不建身份/内容/管理审批/分析业务表。 |
| A04 原子性与权限 | Repository只借用当前连接；固定锁总序、版本条件更新、保存点唯一冲突恢复和同连接主键回读。业务、版本、幂等结果、Outbox、适用审计同事务；PG READ COMMITTED/500ms锁超时，40001/40P01有幂等键才最多重试3次；SQLite WAL/BEGIN IMMEDIATE/5000ms busy timeout，唯一冲突不消耗事务重试预算。应用生成UUIDv7 operation/event ID及统一UTC微秒时间；提交确定成功/确定回滚/未知分离，未知查询绑定原actor/scope/key，无未经鉴权HTTP入口。普通幂等响应30天，到期不清业务唯一事实；其他保留待决不做自动清理。 |
| A04 Outbox/审计 | sink注册及执行，(sink,event_id)去重；领取/续租/过期回收、批次隔离、旧持有者拒绝、退避/DLQ/受控重放及外部停用。PG skip locked集中方言；SQLite取ID批、条件更新并核行数、lease_batch_id回读。standalone与embedded同一调度，SQLite宿主互斥、取消/优雅退出/重启恢复；A05前只轮询。AuditEvent/SensitiveAccessEvent仅追加；高危审计失败回滚，敏感读先授权并提交访问审计再返回；载荷禁止Secret/C3，更正追加，runtime无UPDATE/DELETE/TRUNCATE审计权限。独立权限域复制仅合同/关闭行为/本地适配；Webhook版本模型预留，不开放创建或投递。 |
| `WP-MVP-A02` / 始终In progress | 仅Flag持久化、只读接口、一致快照加载和CLI实际list；依赖A04/A03。唯一Flag注册点不变，新增版本化持久化结构和迁移全关快照并核注册一致；显式ACL只授runtime SELECT。未知项/非法值/损坏数据报错，缺项安全关闭，读失败不可用；默认值与数据库值分列。加载不修复/写回，不增加Flag写入口/环境旁路，不实现set、manage/config、审批、只读/紧急关闭或提示态；这些仍依赖后续授权/A04审计接线/D04。任何业务Flag不开启。 |
| 本轮测试与状态门槛 | 隔离临时SQLite、合成命令、本地sink：新旧迁移/重放/双前缀/注册ACL/SQL lint；共同提交与任一步失败回滚；同异载荷/版本冲突/upsert/错误分类/重试预算；未知提交受控模拟与受限查询；Outbox去重/租约失效/串批/退避/DLQ/重放/停用；审计追加/失败关闭/敏感读授权审计门槛/脱敏；Worker实际消费/退出恢复/宿主互斥；Flag初始全关/重启读取/缺项未知非法失败/CLI；A00/A01环境路由资产及migrate/status/seed回归；M0-05模型一致性。运行dev-check、diff检查、逐文件暂存复核。真实PG/Redis全矩阵、多Worker并发、安全全量、跨平台、性能与视觉留E01/E02/E04；模拟不冒称真实网络故障。A04完整交付及独立复核后才可Implemented；产品需求逐条评估，不批量推进，绝不授Verified或阶段门禁。 |

#### 本轮实际交付、逐需求核对与证据

- 交付版本为本记录所在的本机实现提交：新增`0002_engine`/`0003_flags`配对迁移和受信表/索引/显式ACL，未改0001。沿用全部工具链，仅因UUIDv7与MSRV 1.88可用的OS文件锁新增uuid/fs2及必要锁定依赖，没有升级既有依赖。platform-*仍只依赖platform，不横向依赖；应用组合独立存储与策略。
- A04事务/幂等、Outbox/Worker、追加审计已完成本轮完整交付：共享合同、固定锁序、保存点/版本/主键回读、三次重试、原actor/scope/key结果查询；未知/取消连接明确丢弃，故障注入只在测试；事件和目标sink持久化、租约fence/续租/回收、退避/DLQ/幂等受控重放、进程锁/退出/恢复；审计失败回滚、敏感投影提交后才释放、Secret/C3裁剪、追加更正/关闭复制和本地适配。真实身份授权、业务高危操作、生产独立权限域和供应商仍待后续。操作手册为[基础引擎](../docs/development/engine.md)。
- A02本轮切片完整：14项全关迁移与注册一致、数据库单SELECT快照、默认/当前/版本/缺项分列，未知/损坏拒绝与不可用失败，无修复写回；CLI实际重启读取和缺项测试通过。整包仍In progress，写命令/审批/运行控制/提示页未交付，不启用任何业务Flag。
- M0-05的[派生模型](../docs/design-notes/2026-09-capacity-baseline.md)、机器负载档案、有限NDJSON生成器及A05/E02指标口径完成。两名代理独立交叉复核数值、单位、规模、分布假设、缓存/地区/设备/窗口和分位数口径；只有1000内容行小样本，无百万装载或性能测量。
- 独立复核发现并修复了借用连接事务控制旁路、未知提交连接归池、敏感投影提前释放/错误结果/失败暂存状态、Outbox追加后逆向锁、重放旧operation及测试未接真实审计等问题。开发期`./scripts/dev-check.sh`最终完整通过：141项Rust测试（124单元、17应用集成）、51条HTTP冒烟、真实SQLite迁移、Clippy/rustfmt/SQL lint/handler守卫与diff检查；另10项容量测试及A00冻结检查通过。覆盖旧A03升级至新引擎/Flag、默认及非默认前缀，实际Worker子进程SIGKILL恢复/SIGTERM退出和宿主互斥。首次全检的旧seed三表断言、后续新增测试Clippy告警已修正，失败日志一并保存。
- 证据：[summary.md](../docs/evidence/mvp/foundation-engine/dev-20260905/summary.md)、同目录开发检查日志、模型与冻结复核、validation.json及逐文件暂存审查。dev-check运行时暂存为空；随后已显式暂存并完成逐文件复核，记录见同目录staged-review.json，不能用空暂存替代审查。
- 未执行真实PG/Redis完整矩阵、PG网络中断、多Worker并发、安全全量、跨平台、性能或视觉完整验证；SQLite提交拒绝是真实本地存储错误，未知回执丢失是受控模拟，两者不冒称真实PG网络故障。留E01/E02/E04；生产IAM/SiteOwner/法律/供应商/视觉/保留待决均未批准，无Verified、阶段或发布门禁。

| 精确需求/范围 | 本轮状态裁决 | 交付与剩余依赖 |
|---|---|---|
| `INTEG-004` | Implemented（开发期） | 01原文要求的事务性Outbox可靠源、版本化Webhook事件模型与首版禁止第三方创建均已实现；双后端代码/迁移、SQLite持久化及实际调度证据见上。真实PG硬化留E01，无第三方创建/真实Webhook投递。 |
| `ADMIN-012` | In progress | 本地原子审计、同事务复制请求、关闭行为和本地复制适配已交付；完整高危业务操作接线及生产独立权限域仍依赖B/D04与供应商/权限批准。 |
| `ADMIN-013` | In progress | 类型化审批/前后摘要/关联/时间合同及敏感读取门槛已交付；真实资料查看、证据下载、导出、密钥和break-glass流程仍待身份/管理/领域接线。 |
| M0-05、A02及01 §7.3/7.4/8.3平台约束 | 仅工作包/子范围交付 | 不据基础引擎推进任何身份/内容/管理产品需求；容量目标无实测，不宣称性能达标。其余产品需求状态不变。 |

### 5.6 A05 → A06 平台开发登记（2026-09-05）

项目负责人及最终批准人：项目所有者（用户）；Codex 负责实现、独立交叉复核、开发期验证及本机提交。实施前工作区干净，完整核对 AGENTS、README 与01/02/03/05/06/04/10/20/90/99。DEC-047/048先单独登记提交；MVP可信用户无账号年龄门槛、统计UTC。DEC-049进一步批准正文至少10、回复至少2个英文字符，以及WAU/MAU和果农指数的自然周/月；Unicode折算与自然周起点已询问，依赖计算等待答复，领域有效性与真实指标接线继续后续。保留Workspace与crate方向：共享合同在platform，数据库适配在platform-db，其他platform-*只依赖platform，apps组合和注入。先冻结A05合同，再接A06；不重新搭建A03/A04。

| 设计项 | A05 | A06 |
|---|---|---|
| 精确范围与原文 | 平台任务WP-MVP-A05；01 §7.4/7.5/8.2–8.5、§4.20状态页路由，02 §12/17/18；不交付身份业务限流或A02写入口。 | DATA-001..006、INTEG-001/006与INTEG-005代码面，01 §1.3/4.18/4.19、02 §11/14/17/18；不包括RSS、验证码、密码泄露检查、搜索词典、外链或第三方Webhook。 |
| 依赖 | A02配置/Secrets/启动/Flag只读切片（DEC-047），A03/A04、M0-05；A00本地品牌与冻结组件。 | A04已提交Outbox与Worker，A05冻结异步合同与观测接口；B/C/D领域生产者/看板后续。DEC-011真实供应商、DNS/凭证继续关闭。 |
| 共享接口 | CacheStore/Wakeup/Lease/RateLimiter统一BoxFuture+Result；Disabled/Unavailable/InvalidInput区分；受控PublicCacheKey含版本/public可见性，默认TTL30秒，CacheEntity失效；Wakeup固定Outbox通道、超时等候；Lease持有者令牌续租/释放；RateLimitPolicy显式参数。RedisRuntime配置装配，Environment枚举生成bcm:<environment>:v1，普通读回源/高风险依赖写失败关闭；内存限流只供测试，关闭模式明确无能力。 | 版本化EventDictionary与受信统计窗口/排除上下文，AnalyticsConsumer通过已提交Outbox引用取事实；MailProvider/MessageSource/FeedbackSource、ObjectStore/ImageProcessor/MalwareScanner/ErrorTracker/BasicAnalytics异步Result合同；关闭、本地测试和不可用分开。现有Sink注入，无公开事件/反馈HTTP。 |
| 最小持久数据 | 0004配对迁移operations.status_announcements：公开标题/正文、maintenance/incident、公布及起止UTC时间、版本。只读源为主库，空态和查询失败分离；无自动维护/事故推断。 | 0005配对迁移analytics.event_facts/projection_receipts/aggregates/watermarks：事件与接收时间、字典版本、伪匿名引用、受信排除、已提交Outbox关联；receipt按投影版本/event_id唯一，水位保留乱序/延迟信息。0006配对迁移platform.mail_feedback/mail_suppressions/mail_dispatches/mail_alerts：反馈来源+ID唯一/摘要防异载荷，recipient_ref、分类/抑制原因/硬退信计数/版本、脱敏投递状态及告警。邮箱/正文不入Outbox或日志，由受信消息源回读。 |
| 权限与公告写边界 | status_announcements仅app_rw SELECT，worker/report无授权；写入仅migrator维护数据边界，开发测试用合成夹具，没有新增管理写API或未经鉴权CLI写口。公开/status只投影C0公告和通用状态，不公开连接/拓扑/详细指标；诊断指标仅本机受控读取。 | analytics facts由受信生产服务同事务INSERT，worker SELECT；投影由worker显式SELECT/INSERT/UPDATE/DELETE，report_ro仅脱敏aggregate列。邮件dispatch/suppression为worker SELECT/INSERT/UPDATE，feedback/alerts为worker SELECT/INSERT；app无邮件表权限，report仅dispatch分类/状态/时间与feedback种类/时间列，无recipient或反馈ID。dispatch/feedback显式simulated，本地邮件只报LocalRecorded；告警同事务追加mail.alert Outbox。所有新对象经SchemaRenderer受信注册与显式ACL，mysql8占位，不创建身份/内容/IAM/审批表。 |
| 事务、版本与Outbox | 公共缓存只有确定提交后可失效；已有Outbox目标提供可靠重试，Redis只是缓存/短租约/唤醒，数据库租约/去重仍权威。Worker在丢失/断连/关闭时固定轮询恢复，取消与退出有界；namespace清理仅当前环境SCAN+UNLINK。 | 事件事实与Outbox同事务，消费者去重/聚合/水位同事务；版本条件与固定锁序、失败回滚、重放重建不重复累计。邮件等副作用只走已提交Outbox/Worker；反馈去重+抑制+告警同事务，营销退订不屏蔽安全事务邮件，硬退信永久失败停止重试并告警；生产率/延迟阈值仅Option配置合同，无猜测默认。 |
| 观测与本地输出 | 结构化脱敏日志与trace贯穿HTTP/事务重试/Outbox/Worker/sink；固定标签HTTP/池/重试/lag/积压/租约/DLQ/缓存/依赖错误，采集值、未支持、未接入分离，不用零冒充缺失；邮件/媒体/搜索同接口。liveness/readiness/依赖降级分离，可选Redis错误不阻断公开读。 | 本地邮件生成可检查品牌邮件文件；本地对象实际隔离读写，图片真实解码/预算/重编码清元数据，模拟扫描明确非真实检测；本地错误/基础分析生成脱敏结果。对关闭/故障不报已发送/已扫描/已上传，真实provider与全部业务Flag关闭。 |
| 依赖新增理由 | Redis客户端仅为Redis8/Tokio/Rustls网络适配；复用已有Tokio和tracing，结构化日志启用既有subscriber JSON，不升级无关依赖。 | image仅为本地PNG/JPEG真实解码/限像素/重编码，不用固定成功替代图片处理；其余优先复用已有依赖。 |
| 开发验证与复核 | 异步合同/装配/禁用不可用/命名空间/TTL/失效/令牌过期/唤醒丢失轮询/取消，日志脱敏/关联/采集/健康及/status空故障；提供真实Redis8可复现测试入口但本轮不假称已执行真实合同。 | 合成事件+真实临时SQLite去重/排除/边界/延迟/聚合/水位/重建；本地适配器实际输入输出/失败关闭；邮件分类/反馈去重/抑制/硬退信停止重试告警。交叉独立复核接口、Redis故障/敏感日志、分析正确性和邮件规则后修复。 |
| Flag与状态条件 | 14项业务Flag保持关闭，Redis测试只隔离配置，A02整包始终In progress。全部约定开发交付、最小集和独立复核后A05才可Implemented，不授Verified。 | DATA/INTEG逐条裁决，合成测试不证明真实领域生产者/看板完成；平台切片完成但领域接线缺口则A06整包In progress。未批准口径/阈值暂停依赖计算，继续显式参数合同与合成处理。 |

共同最小集设计：适用单元、真实临时SQLite冒烟、./scripts/dev-check.sh、git diff --check、逐文件暂存复核。原迁移/种子/Flag list/事务审计/Worker恢复/A00/A01回归；无独立构建或完整资产构建。真实PG/Redis全矩阵、多实例、安全、跨平台、性能与视觉留E01/E02/E04。AGENTS.md、g.md、本机配置/Secrets/数据库/缓存/处理产物仍忽略，不做保留期或自动清理批准。以下登记本轮实际结果。

#### 实际交付、逐需求裁决与证据

- A05 完成本轮开发交付：四个异步 Result 合同、Redis8适配/受控环境namespace、默认30秒公开版本缓存/失效、令牌短租约、共享限流及关闭/失败分流；确定提交提示和已有Outbox缓存失效路径，丢提示/持续断连仍轮询。JSON日志、请求至sink及子事件trace、有限HTTP/DB/Worker/sink分组、秒制series与显式ms汇总、进程重置标识、真实App/Worker池、事件与delivery分别计数、缺失状态、健康和真实C0公告读路径已接线。没有新身份、管理写口或A02运行控制；[Redis](../docs/development/redis.md)、[健康与观测](../docs/development/observability.md)给出复现和边界。
- A06 平台切片已接已有Worker：版本化七事件字典、受信排除、已提交来源校验、投影根锁/持久receipt/聚合/水位共同事务、迟到事件、脱敏窗口原语与原子重建；六类关闭/本地异步适配，真实隔离文件/PNG与JPEG处理、明确模拟扫描、品牌MIME与本地JSON。邮件反馈持久去重、营销退订不屏蔽安全邮件、硬退信停止重试及同事务Outbox告警、模拟/真实分组和显式告警配置。操作说明见[分析](../docs/development/analytics.md)、[适配器与邮件](../docs/development/adapters.md)。真实领域来源未接线时明确Disabled；不把本地邮件标Sent。
- 数据/依赖：只新增0004/0005/0006三对PG/SQLite迁移、受信注册及列级最小ACL，旧迁移未改；mysql8空占位。新引入redis1.6.0、image0.25.10及必要传递依赖，subscriber启用JSON；锁文件无已有包移除或升级，crate无横向依赖。PG内嵌Worker启动拒绝与02体验模式一致，避免Web承担Worker权限。A00原模板/资产哈希未变；冻结检查仅明确允许新status模板，并限制它复用原类名，不改视觉基准。
- 最小集完整通过：182项Rust测试、55条HTTP冒烟、真实临时SQLite六迁移/重放/双前缀/种子/Flag list/审计/Worker恢复、Clippy/rustfmt/SQL lint/handler守卫/diff；1项真实Redis8合同测试明确ignored、未执行。另10项容量模型测试与A00冻结检查通过。已读取实际合成MIME（plain/HTML/CID品牌）和对象输出；原始处理产物在Git忽略的data/adapter-probe，本仓库只存摘要。增量复核和最终逐文件暂存审查见证据。
- 三名代理交叉独立复核后修正Redis时间精度/断连取消、分析来源校验/SQL拼接、图片元数据预算/符号链接/放大预算、邮件窗口/抑制状态/回执丢失重放、audit停用错误分类、PG公告schema权限、trace继承、状态故障文案、指标有限分组/池用途/事务取消与RetryExhausted。现无未关闭的本轮代码阻断。早期编译、过期迁移断言、转义断言、配置测试与SQL lint失败保留在证据中，不伪造一次全过。
- 版本为本节证据所在的 `新增：交付 A05 平台服务及 A06 分析邮件底座` 本机提交；决策先行提交6fa32f7与bb0b584。证据：[summary.md](../docs/evidence/mvp/platform-services/dev-20260905/summary.md)、同目录validation.json、开发检查/失败日志、独立复核及staged-review.json。开发检查的空暂存阶段不代替最后显式暂存后的逐文件复核。
- A02继续In progress，A06整包继续In progress，14项业务Flag全关；不授Verified或阶段/发布门禁。未执行真实PG/Redis集成、真实provider/DNS、生产IAM、安全/并发/跨平台/性能/视觉完整矩阵；精确数据库commit-to-sink、PG专项统计、完整客户端延迟、真正业务埋点与看板仍明确未接入。保留期、供应商、生产阈值、法律与正式视觉等Pending不被本轮批准；自然周起点和Unicode折算按DEC-049等待用户答复。

| 精确需求 | 本轮状态裁决 | 实际交付与剩余依赖 |
|---|---|---|
| `DATA-001` | In progress | 注册/验证/邀请/阅读/回复/有效贡献字典和去重基础已交付；首次行为判定、真实生产者及漏斗展示仍依赖B/C/D。 |
| `DATA-002` | In progress | UTC日/月窗口与脱敏去重主体原语已交付；WAU/MAU/果农指数自然周/月已批准，真实活跃/有效性/分层计算和看板未交付，字符折算与自然周起点待明确。 |
| `DATA-003` | In progress | 主题/回复事件投影、迟到恢复和重建已有测试；有效回复、零回复率和首次回复时间需要内容事实及领域接线。 |
| `DATA-004` | In progress | 固定版块/标签/渠道/设备维度与聚合合同已交付；真实受信目录、生产事件和管理统计未接线。 |
| `DATA-005` | Implemented（开发期） | 版本化字典、生产者/发生/接收时间、UTC、去重/排除、处理延迟与重建来源均有代码和操作说明；开发合同测试通过，不据此宣称真实埋点上线或延迟达标。 |
| `DATA-006` | In progress | 四类排除来自不可HTTP反序列化的受信合同，持久消费及合成测试通过；真实身份/管理员/机器人/活动事实读取及管理选择尚未接线。 |
| `INTEG-001` | Implemented（开发期） | 可替换合同、明确关闭/不可用及六类可检查本地实现，实际Worker输出通过；符合未批准供应商时的本地交付边界，真实provider与领域业务仍关闭。 |
| `INTEG-005` | In progress（代码面已交付） | 事务/营销分类和适用头、品牌MIME、送达/延迟/退信/投诉及配置告警已实现；发送域SPF/DKIM/DMARC、真实投递和反馈接入继续DEC-011。 |
| `INTEG-006` | In progress | 持久抑制/反馈去重、营销与安全隔离、硬退信永久停止和告警完整；真实账号偏好/退订入口、provider反馈鉴权/接线未交付，不以模拟证明真实用户流程完成。 |

## 6. 门禁状态

| 门禁 | 当前状态 | 原因 |
|---|---|---|
| M0 Repository Ready | In progress | `WP-M0-01`、`WP-M0-03`、`WP-M0-04` 已为 `Implemented`；`WP-M0-02` 受 `DEC-019` 阻塞，`WP-M0-05` 模型已交付，`WP-M0-06` 尚未完成，统一硬化证据和阶段门禁尚未通过。 |
| M0 Production Approved | Blocked | 多项决策、人员和生产事项待定 |
| MVP Repository Ready | In progress | `WP-MVP-A00/A01/A03/A04/A05` 已完成开发期交付，`A02/A06` 仍为部分交付，其余 MVP 工作包及统一硬化尚未完成；DEC-045/046本机版本化不代替正式门禁。 |
| MVP Production Approved | Blocked | 依赖 M0、真实依赖和 18 条门槛 |
| P1-A/P1-B/P2/Later | Closed | 前置阶段和 `DEC-014` 未满足 |

## 7. 证据索引规则

- `99` 只链接每个候选的最新 manifest，不粘贴长日志。
- 历史候选在 `docs/evidence` 中保留不可变目录和 SHA-256。
- 同一命令的新运行创建新候选，不覆盖旧证据。
- 失败报告同样登记，修复后链接新报告并保留失败历史。
- `/tmp` 可以保存运行中输出，但必须生成可复核 manifest 才能支持 Verified。
- 开发期只登记最小集运行结果；完整验证证据在统一硬化阶段补齐（`DEC-020`）。
- 本文件是唯一执行状态登记处。根目录设计稿已按 `DEC-026` 退役为 `docs/design-notes/2026-09-rust-skeleton.md`，其第 5 节记录的实施进度已并入本文件第 5 节；仓库不再存在第二套执行状态。

## 8. 状态变更审查

每次状态提升检查：

1. 需求原文是否全部完成，而非只有底座。
2. 页面/API、数据库、权限、Outbox 和测试是否一致。
3. 默认及非默认 schema 是否真实运行（仅 `Verified` 及以上晋升时适用；开发期晋升 `Implemented` 不适用）。
4. 页面是否完成适用浏览器和视觉验证（仅 `Verified` 及以上晋升时适用；开发期晋升 `Implemented` 不适用）。
5. 后续 Feature Flag 是否仍关闭。
6. evidence manifest 是否绑定当前 commit（仅 `Verified` 及以上晋升时适用；开发期晋升 `Implemented` 不适用）。
7. 计划文档内部引用与状态是否一致。

状态纠正可以降低状态，但不能降低需求。发现证据与状态不一致时必须立即纠正，并记录原因。
