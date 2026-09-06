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

下列待决决策直接定义某些需求的行为，未批准前对应需求记 `Blocked` 而非 `Implemented`；它们阻塞的是 Repository Ready，不只是 Production Approved。2026-09-06 已关闭三项：`DEC-017`（由 `DEC-056` 批准，`CAT-014` 解除 `Blocked`）、`DEC-019`（由 `DEC-057` 批准，`05` §15 门槛获得参照）、`DEC-053`（由 `DEC-054`／`DEC-055` 批准，内容产品参数不再暂停依赖分支）。剩余阻塞：

| 待决 | 受阻需求 | 说明 |
|---|---|---|
| `DEC-011`／`CR-014` 密码/挑战provider | `AUTH-005`、`AUTH-007` 的真实provider与语料 | DEC-051已批本地筛查、风险挑战与失败关闭；真实供应商/语料来源和更新仍后续，本地能力裁决见5.8。 |
| `DEC-011` 外部 provider | `EDIT-009`、`EDIT-013` | 对象存储与恶意扫描 provider 未选 |

判定规则见 [`README.md`](README.md) 第 6 节：外部 provider 依赖类需求若 adapter、故障语义、配置开关与本地测试实现均已交付且能力按已批准的未决行为保持关闭，可记 `Implemented`；行为完全由待决决策定义的需求记 `Blocked`。

## 5. 工作包状态

| 工作包范围 | 当前状态 | 证据与剩余工作 |
|---|---|---|
| `WP-M0-01` | Implemented（开发期，2026-09-05） | 执行责任：Codex；批准责任：项目所有者（用户）。已冻结并交叉复核 `01` 的 316 个唯一需求及阶段映射、18 条 MVP 门槛，`04` 的工作包与依赖，以及 `06` 的决策、待决事项、冲突和风险；所有当前 Pending 均有责任角色、阻塞范围和未决定时行为。开发期复核覆盖需求计数、阶段计数、过期口径扫描、`./scripts/dev-check.sh`、`git diff --check` 与暂存区复核；状态止于 `Implemented`，不据此授予 `Verified` 或任何发布门禁。 |
| `WP-M0-02` | In progress | `DEC-057`（2026-09-06）已批准 `visual-baseline-v1` 组合方式并指定以 `WP-MVP-A00` 产出冻结，`DEC-019` 关闭；剩余动作为产出可复核的基准冻结记录并在本文件登记。像素与几何门槛仍在 `WP-MVP-E04` 硬化阶段验证。 |
| `WP-M0-03` | Implemented（开发期，2026-09-05） | 技术与供应链基线已交付：保留 Rust/Cargo、共用 Node/npm/esbuild 锁文件、固定测试镜像与扩展输入、扫描配置及复现入口。负责人、依赖、交付物、测试设计与证据见第 5.2 节；沿用现有 Workspace 与 A00，未授予 Verified 或推进任何阶段/发布门禁。 |
| `WP-M0-04` | Implemented（开发期，2026-09-05） | 完成15逻辑schema/受信注册、数据库角色与ACL、迁移流程/元数据、固定锁序、幂等/Outbox/追加审计及16类保留矩阵派生设计；独立复核与最小集通过，责任、依赖、交付和证据见第5.4节。A04引擎当前交付见第5.5节，法律/IAM待决保持后续边界，不推进产品需求或正式门禁。 |
| `WP-M0-05` | Implemented（开发期，2026-09-05） | 模型、负载档案、确定性生成方式、单位与分位数口径及独立复核完成；10项模型测试，未灌百万数据或实测，见第5.5节。 |
| `WP-M0-06` | Not started | 冷启动与合规尚未交付。 |
| `WP-MVP-A00` | Implemented（开发期重做，2026-09-05） | 14个隔离SSR夹具、30项本地源资产、模板结构/CSS类名冻结、17张本机原始参考截图和开发期检查已交付，详见第5.1节。按用户修改后的 `05` 与 `DEC-045` 如实登记未知许可并本机版本化；`DEC-019` 已由 `DEC-057` 批准并指定以本包产出冻结基准，E04 完整视觉矩阵仍未执行，不授予 `Verified`。代码版本为本条记录所在实现提交。 |
| `WP-MVP-A01` | Implemented（开发期，2026-09-05） | 沿用16成员Workspace、三个可执行与七步dev-check；复用 M0-03 锁定的 Node/npm/esbuild 和根 npm 锁文件，完成真实资产构建、30项 SHA-256 产物与 manifest、Rust 加载校验/SSR helper、immutable 指纹路由、生产隔离及操作手册。原A00冻结未改，模板仅11处资源URL接线。5项构建测试、15项web测试、四环境492次HTTP/84个SSR变体/8次启动拒绝、最小集72个单元测试和51条HTTP冒烟、diff与暂存区逐文件复核通过；SQLite迁移明确跳过（A03）。本轮开发交付无剩余缺口，代码版本为本条证据所在的 `新增：完成 WP-MVP-A01 静态资产构建与指纹服务` 提交；完整说明、依赖、测试与证据见第5.3节。E01/E04完整矩阵和阶段/发布门禁仍未通过。 |
| `WP-MVP-A02` | In progress（开发期，2026-09-05） | 本轮配置前置完成：三程序共用安全默认/主文件/local/环境/Secret分层，四环境模板、严格解析、连接/前缀/Redis/Worker校验、Secret脱敏、PostgreSQL启动检查与手册；配置13项及CLI/连接回归随最小集通过。本轮另完成版本化Flag存储、一致只读加载与实际CLI list（第5.5节）；整包剩余受控flag set与/manage/config、审计接线/审批、只读/紧急关闭及提示态，依赖后续授权/A04/D04；14个MVP Flag仍默认全关。配置前置见第5.4节。 |
| `WP-MVP-A03` | Implemented（开发期，2026-09-05） | 已修复有效轨为空豁免；交付PG/SQLite真实迁移、15逻辑schema映射与3张必要初始表、专用单连接/前缀锁/元数据/漂移拒绝/失败回滚/FK恢复、受信表注册、PG组角色bootstrap与显式/默认ACL、实际migrate/status/seed、4版块规则占位和共享SQL lint。迁移20项及种子/ACL/CLI回归、真实SQLite与HTTP冒烟通过；角色与IAM区分，mysql8只占位，真实PG ACL/并发等留硬化。版本、设计和证据见第5.4节。 |
| `WP-MVP-A04` | Implemented（开发期，2026-09-05） | 事务/幂等/未知提交、Outbox调度/受控重放、追加审计/敏感读取、双宿主恢复与Webhook版本模型完成；独立复核及真实SQLite最小集通过，见第5.5节。无真实provider或授权路由，不授Verified。 |
| `WP-MVP-A05` | Implemented（开发期） | Redis异步合同、故障/轮询、有限观测、真实健康与C0状态公告；开发期验证和独立复核通过，代码/测试/证据见第5.6节。依赖按DEC-047细化，真实服务与完整门禁留硬化。 |
| `WP-MVP-A06` | In progress | 分析持久流水线与身份/主题/回复原始事件、正常Worker source/Secret接线已接；5.9含内容幂等/排除/重启验证，其余领域、首次行为/有效统计及看板仍缺。 |
| `WP-MVP-B01` | In progress | DEC-051身份流程与正常宿主已接；AUTH-003内容事务调用见5.9，INVITE-006按信任调额仍缺，不将内容切片视作整包完成。 |
| `WP-MVP-B02` | Implemented（开发期） | AUTH-004..007按DEC-051及README §6完成认证/恢复/会话/通知、四维限流、密码筛查与本地挑战adapter；真实provider保持后续、生产关闭，证据与精确边界见5.8。 |
| `WP-MVP-B03` | Not started | MFA、管理会话和AUTH-014首账号提权未纳入本轮。 |
| `WP-MVP-B04` | In progress | USER-004/005/007、真实基础资料与5.9公开/本人主题回复列表接线；作者隐私待决，上传、其他集合/关系、偏好、导出及注销/删除传播仍缺。 |
| `WP-MVP-B05` | Not started | 新用户引导和首次贡献未接入。 |
| `WP-MVP-B06` | In progress | 身份与内容同事务信任/限制/风险接线、参数注入及四维故障合同已实现；内容政策与授级取值已由DEC-054批准但未装配，挑战、重复文本与其他领域/完整处置仍缺。 |
| `WP-MVP-C01` | In progress | 真实四视图/导航/SSR/签名分页；作者隐私、统计、搜索及正式视觉仍缺，见5.9。 |
| `WP-MVP-C02` | In progress | 真实内容/楼层/修订/本人维护/事务与权限合同；正式发布政策、完整管理转换及统计仍缺，见5.9。 |
| `WP-MVP-C03` | In progress | 安全Markdown和预览已接；完整编辑器、草稿、上传及媒体全链仍缺，见5.9。 |
| `WP-MVP-C05` | In progress | 只借入同事务权威search_documents；搜索引擎/页面、SEO、Feed/sitemap等仍缺，见5.9。 |
| `WP-MVP-C04` | Not started | 互动、通知与跨设备阅读位置后续。 |
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
| `DATA-002` | In progress | UTC日/月窗口与脱敏去重主体原语已交付；WAU/MAU/果农指数自然周/月已批准，字符折算与自然周起点已由DEC-055批准（1标量=1字符、ISO-8601周一起点）但未接入计算，真实活跃/有效性/分层计算和看板未交付。 |
| `DATA-003` | In progress | 主题/回复事件投影、迟到恢复和重建已有测试；有效回复、零回复率和首次回复时间需要内容事实及领域接线。 |
| `DATA-004` | In progress | 固定版块/标签/渠道/设备维度与聚合合同已交付；真实受信目录、生产事件和管理统计未接线。 |
| `DATA-005` | Implemented（开发期） | 版本化字典、生产者/发生/接收时间、UTC、去重/排除、处理延迟与重建来源均有代码和操作说明；开发合同测试通过，不据此宣称真实埋点上线或延迟达标。 |
| `DATA-006` | In progress | 四类排除来自不可HTTP反序列化的受信合同，持久消费及合成测试通过；真实身份/管理员/机器人/活动事实读取及管理选择尚未接线。 |
| `INTEG-001` | Implemented（开发期） | 可替换合同、明确关闭/不可用及六类可检查本地实现，实际Worker输出通过；符合未批准供应商时的本地交付边界，真实provider与领域业务仍关闭。 |
| `INTEG-005` | In progress（代码面已交付） | 事务/营销分类和适用头、品牌MIME、送达/延迟/退信/投诉及配置告警已实现；发送域SPF/DKIM/DMARC、真实投递和反馈接入继续DEC-011。 |
| `INTEG-006` | In progress | 持久抑制/反馈去重、营销与安全隔离、硬退信永久停止和告警完整；真实账号偏好/退订入口、provider反馈鉴权/接线未交付，不以模拟证明真实用户流程完成。 |

### 5.7 B01/B02/B04 身份开发登记（2026-09-05，DEC-050）

产品决策与最终批准：项目所有者（用户）；实现、独立复核、开发验证与本机提交：Codex。开始前工作区干净，已完整阅读 AGENTS 并从 README 核对产品、技术、权限、视觉、决策、路线/阶段、质量与既有追踪。DEC-050 单独提交后实施；不推送、不部署、不发送真实邮件。

| 设计项 | 本轮精确合同与完成条件 |
|---|---|
| 需求/状态 | B01：AUTH-001/002/003/008/010、INVITE-001/002/003/005/006；INVITE-004 模型/受限服务。B02：AUTH-004..007，未决泄露检查/挑战子句保留缺口。B04：USER-004/005/007 与 USER-001 基础展示；B06：TRUST-001、RISK-001/002/003/007/008 身份子范围；A06：DATA-001/006、INTEG-005/006 身份邮件/事件接线；MOD-010 只读切片。此登记不宣称实现完成。 |
| 依赖/交付 | DEC-050 细化 A06/B06/MOD-010；沿用 A00/A01/A03/A04/A05 和 A02 配置/Secrets/Flag只读底座。交付共享合同、身份 core/repo/HTTP/SSR、最小配对迁移、受信风险服务、邮件源与本机操作/恢复说明、开发证据。 |
| 数据 | identity 账号/凭证/资料、普通会话、一次性令牌、邀请及额度、用户名历史、同意与邮件请求引用；trust 初始快照/风险/邀请归因；moderation 政策版本读取。仅必要 PG/SQLite 新迁移和最小 ACL/受信注册，旧迁移不改，mysql8 占位。令牌/邀请码哈希，原始秘密不入Outbox/日志/明文任务表。保留期不自选，用户名历史90天按DEC-030。 |
| 接口 | 注册/验证/登录/退出/找回/重置/重新认证；本人简介/邮箱/用户名/密码/设备与邀请；基础公开资料只投影已接数据。政策服务共享合同由应用注入，跨域写只经当前事务core服务，不增加领域反向或platform-*横向依赖。 |
| 权限 | 服务端普通账号状态与会话/版本、owner、CSRF、5分钟重新认证；改邮箱/密码撤销全部旧会话，30天绝对/14天滑动期限。只读受信上下文，不接受客户端角色/等级/排除依据。T1和邀请权不自动授予，无MFA/管理会话/首账号提权入口。 |
| 事务/版本 | 固定锁序：幂等→identity账号/凭证/会话/邀请额度/邀请→trust→必要政策读取→追加阶段。注册邀请消费、账号凭证资料、三政策同意、初始信任事实、幂等结果和Outbox共同提交；唯一约束和版本条件阻止重复消费/覆盖。提交未知不报成功；跨域写服务不自行提交。 |
| Outbox/分析 | 邮件仅引用受信消息源，现有Worker+持久抑制+本地provider；验证/恢复/邮箱变更/安全通知可靠重试。真实注册/验证/邀请消费事件和排除依据同事务，持久去重沿用A06；RiskEvent版本/规则/最小信号/可解释结果，不存原始IP/设备/邮箱/令牌。首次阅读/回复/贡献/管理看板后续。 |
| Flag/故障 | 所有业务Flag全关；隔离测试库受控启用路由，不增Flag写口/绕过。Redis关闭明确无速率保护；已启用但不可用高风险失败关闭。未定资格/限流/挑战/规范化/邮件令牌边界只暂停依赖分支。 |
| 开发测试 | 单元与真实临时SQLite：注册共同提交/回滚/重放/重复消费/过期撤销；令牌单次/枚举固定成本/CSRF/会话过期撤销；邮箱确认/密码撤旧会话/owner；用户名唯一/冷却/历史/反冒充；邮件实际输出/抑制/失败重试/重启；事件同事务/持久去重/排除/敏感日志；全关/限流故障/A00/A01/A03/A04/A05与Worker回归。运行./scripts/dev-check.sh、diff与逐文件暂存检查。 |
| 独立复核/完成 | 独立代理交叉复核认证/权限/事务/邮件令牌/敏感日志，修复后提交。逐需求按完整条款裁决；B01/B02仅本轮约定范围完整且无未处理缺口才Implemented。A02/A06/B04/B06仍In progress。不授Verified，真实PG/Redis、安全/并发/性能/视觉完整矩阵留E01/E02/E04。 |

历史待询问（已由DEC-051答复的事项见5.8）：T0/T1实际资格及邀请授权、邀请有效期/日界线/额度释放、四维限流数值与网络段/挑战、用户名邮箱规范化/合法性/反冒充/简介限制、新邮箱令牌期限，以及哈希存储与可恢复邮件的派生令牌边界。已集中询问用户，等待答复期间只推进不依赖这些取值的合同；DEC-011/CR-014 与 DEC-049 未决继续保留。

#### 5.7.1 24360bf交付与逐需求裁决（历史，本轮以5.8为准）

代码、负责人、合同和开发证据对应本次身份实现提交；依赖细化先以 `236ba57`（DEC-050）单独提交。开发记录见 [身份切片证据](../docs/evidence/mvp/identity/dev-20260905/summary.md)，操作/恢复见 [identity.md](../docs/development/identity.md)。以下 `Implemented` 均仅指开发交付，不授予 `Verified`；未列需求沿用原状态。

| 需求 | 当前状态 | 实际交付及剩余条件 |
|---|---|---|
| `AUTH-001` | In progress | 邮箱/用户名/Argon2id密码注册、SSR和真实事务已实现；正式规范化、预认证期限、正常装配仍待确认。 |
| `AUTH-002` | In progress | 有效邀请条件、账号/同意/信任/Outbox共同提交，回滚/重放/重复消费已验证；邀请有效期/资格及令牌方案仍待决，不能整条关闭缺口。 |
| `AUTH-003` | In progress | 24小时单次邮箱令牌、验证事实和分析事件已实现；令牌可靠重试方案/宿主装配待批，发帖回复检查接线仍依赖C02/B06。 |
| `AUTH-008` | Implemented | 三份当前政策ID由受信事务读取并核对，注册明确勾选并共同保存同意；开发测试使用标注的合成版本。MOD-010正式文本/发布审批另留后续。 |
| `AUTH-010` | Implemented | 已知/未知、错误密码、无效账号与限流路径统一公开失败；实际Argon2固定工作次数、CSRF前置、注册邮箱独立桶防别名映射有回归。完整时序/攻击矩阵留硬化。 |
| `INVITE-001` | In progress | 256位随机码、哈希、单次/过期/撤销与受限生成服务已实现；邀请码期限和资格待确认。 |
| `INVITE-002` | Implemented | 创建/使用主体、时间、来源、状态与消费事实持久保存；owner投影不返回受邀邮箱/IP/设备或原始码。 |
| `INVITE-003` | In progress | `/me/invites` 提供额度、记录、仅一次显示原始码和本人撤销；日界线及过期/撤销占用规则待确认。 |
| `INVITE-004` | In progress | 必要来源/冻结状态模型和本人受限服务；管理员批量创建/撤销/冻结/查询及管理UI未实现，留D04。 |
| `INVITE-005` | Implemented | 邀请/身份页no-store/noindex/no-referrer，无OG；Outbox/分析/风险/错误和观测不含邀请码，原始码只在受控响应展示。 |
| `INVITE-006` | In progress | 每日5/未用5及当前事务配额检查、跨日未用额度测试已交付；日界线/释放规则、按信任调整政策待确认。 |
| `AUTH-004` | In progress | 登录/退出/找回/重置、密码与邮箱变更安全通知、30天绝对/14天空闲会话及5分钟重新认证已实现；正常策略与邮件源/Secret装配仍缺。异常登录通知属未接触发能力，不靠存在模板宣称完成。 |
| `AUTH-005` | In progress | Argon2id、8–256字符和可注入更高下限已实现；泄露检查按DEC-011/CR-014显式关闭，对应子句未完成。 |
| `AUTH-006` | In progress | 复用A05四维服务、固定账号主体、关闭无保护/已启用故障失败关闭已实现；预算/窗口/网络前缀和生产参数未批准。 |
| `AUTH-007` | Blocked | 仅风险Challenge/不可用合同，未连接人机验证；触发、供应商和故障语义待DEC-011/CR-014，未自选正常流量放行规则。 |
| `USER-001` | In progress | 公开资料仅真实用户名/简介/加入时间/信任快照与已有预设组件；角色领域、主题、回复、贡献统计未接入且不填合成数字。 |
| `USER-004` | In progress | 简介、确认新邮箱、版本/owner/重新认证、旧会话撤销及双邮箱通知同事务；简介/邮箱规范、确认期限和邮件装配待决。 |
| `USER-005` | In progress | 唯一占用、30天冷却、90天历史保留防抢注和并发版本条件；正式用户名合法性/反冒充规则未批准，合成策略测试不能替代产品规则。 |
| `USER-007` | Implemented | 设备粗分类/创建与最近使用摘要、当前设备识别、本人单设备/其他设备退出；事务内重读与owner/CSRF/重新认证检查、撤销后业务拒绝和受限结果查询有测试。 |
| `TRUST-001` | In progress | pending/T0/T1版本化快照、验证事实和服务端邀请资格检查已交付；实际T0/T1资格/发布检查/授信仍待决或后续接线，不自动升级。 |
| `RISK-001` | In progress | 注册/登录/邀请RiskEvent v1与真实业务事务接入；发帖/回复/反馈未接。注册风险与成功分析事实通过同event_id受信关联。 |
| `RISK-002` | In progress | 规则版本、命中摘要、动作/原因、持久去重与冲突拒绝合同已交付；实际规则与其他领域决策仍待接线。 |
| `RISK-003` | In progress | 受信账号、peer IP、设备、邀请码的独立HMAC摘要边界已接，网络摘要未配置时缺省；组合/时间序列规则和网络前缀未批，无单IP自动封禁。 |
| `RISK-007` | In progress | Risk/HTTP/日志投影最小化、独立伪匿名标识、敏感Debug脱敏；正式政策披露与保留期限未交付。 |
| `RISK-008` | In progress | 风险未配置/不可用在身份事务留存503结果，公共读取不调用风险服务；其他高风险领域尚未上线接线。 |
| `MOD-010` | In progress | 仅借当前三政策版本读取/校验、合成与生产隔离；正式法律文本、发布、审批与后台留后续。 |
| `DATA-001` | In progress | 三个真实身份事务生产者已接A06，Worker实际投影；首次阅读/回复/贡献及完整漏斗未接。 |
| `DATA-006` | In progress | 注册来源的受信分析排除位默认true，三事件共同读回，客户端不能自报；系统/治理/作弊等其他领域依据待接。 |
| `INTEG-005` | In progress | 身份邮件源与真实Scheduler本地输出、失败重建恢复已验证；正常Worker仍UnwiredMailSource，待批准令牌/Secret装配，真实发送域与供应商仍留DEC-011。 |
| `INTEG-006` | In progress | 身份规范化邮箱稳定抑制引用、投诉抑制阻止实际本地输出及重建恢复已验证；硬退信复用A06既有规则与测试；真实偏好/退订/反馈鉴权等仍缺。 |

24360bf时正常应用没有注入资格/规范化/令牌密钥/风险参数，所有Flag默认关闭，无配置绕过或Flag写入口。试验性HMAC派生方案仅在受控普通账号测试中装配；它不是产品批准或运行时交付。待答复还包括预认证有效期、新邮箱期限与同目的重发撤销规则；数据/密钥保留与永久失效恢复不从令牌TTL擅自推导。B01/B02/B04/B06/A06/A02均保持In progress；未处理缺口未被其他切片测试抵销。

### 5.8 DEC-051 身份默认规则与正常装配续接（2026-09-05，实施前登记）

用户明确参数及“按照正常定”授权见DEC-051，取代第5.7节中相应的待产品答复；此前`24360bf`保留为受控切片证据，不视为本轮装配已完成。开始前Git工作区干净。负责人：项目所有者决策/最终批准，Codex实现、独立复核、开发验证和本机提交。

| 设计项 | 本轮范围/完成条件 |
|---|---|
| 精确需求/依赖 | 续接AUTH-001/002/003/004/005/006/007/008/010、INVITE-001/002/003/005/006及004受限切片、USER-001基础与004/005/007；TRUST-001、RISK-001/002/003/007/008身份边界；DATA-001/006、INTEG-005/006与MOD-010读取，依赖A00/A01/A03/A04/A05和DEC-050交付。 |
| 交付/接口 | 正式ApprovedIdentityPolicy/资格、1–7天邀请与幂等、验证重发/新旧令牌、跨表单预认证、默认四维预算与组合挑战、本地密码语料合同、正常Web/内嵌/独立Worker Secret及邮件source装配、实际SSR路由。原始令牌不入DB/Outbox/日志。 |
| 数据/权限 | 保留0007；如挑战需持久事实则新增0008 PG/SQLite配对与受信注册/最小ACL。账号active+verified决定邀请；不授T0/T1/特权。普通会话owner/CSRF/重新认证/凭证版本继续事务内重读。挑战只绑定预认证、动作和指纹，5分钟单次，关闭/失败不放行。 |
| 事务/Outbox | 原子注册/同意/初始事实/邮件分析事件不拆分；同用途旧token失效与新token/mail/Outbox共同提交，重复同key不新增。密钥版本保持不可变源重建；邮件抑制与投递状态沿用A06。邀请天数和挑战证明绑定实际请求，不可跨主体/动作/载荷复用。 |
| Flag/配置 | 14项业务Flag默认全关，仅隔离DB受控启用验路由。无Flag写入口/运行时绕过、无首特权引导。Secret经既有分层注入，配置/日志脱敏；本轮只LocalTest provider，staging/production拒绝模拟模式。 |
| 测试/完成 | 真实批准policy而非SyntheticPolicy测试规范边界、角色冒充、邀请3/7天/撤销库存/UTC额度、不自动授信；重发旧失效/单次与幂等、跨表单/Unknown查询；四维/挑战单次与故障、离线密码语料失败；实际应用装配SQLite全链路、Outbox/邮件抑制/重启/keyring恢复；适用回归、真实临时SQLite、./scripts/dev-check.sh、diff和逐文件暂存复核后中文提交。 |
| 剩余门禁 | 正式政策文本/发布审批、T级授予流程、内容调用、真实provider/语料来源、完整A02/A06/B04/B06及E01/E02/E04保持后续；本轮参数不再反复询问。B01/B02仍逐项审核后才能Implemented，无Verified/发布门禁。 |

#### 5.8.1 DEC-051交付与逐需求裁决（历史，后续内容状态以5.9.1为准）

依赖决策`236ba57`、受控身份切片`24360bf`保持；本轮默认规则先以`18808a7`单独提交，令牌桶语义澄清另以`44cb95b`提交，再实施代码。负责人仍为用户产品/最终批准、Codex实现/验证/提交；认证、HTTP及策略代理互相复核，root复核共享装配。开发证据见[DEC-051身份续接](../docs/evidence/mvp/identity/dev-20260905-defaults/summary.md)，本机配置和恢复见[identity.md](../docs/development/identity.md)。第5.7节是历史切片，不再代表当前待决。

| 需求 | 当前状态 | 实际交付与剩余边界 |
|---|---|---|
| AUTH-001 | Implemented | 正式用户名/邮箱规范、反冒充、Argon2id及SSR已接；实际Web工厂/临时SQLite验证，不再依赖Unapproved策略。 |
| AUTH-002 | Implemented | 有效邀请、账号/凭证/同意/初始信任/邮件和分析Outbox同事务；新规则下回滚、重放、重复消费和库存边界有测试。 |
| AUTH-003 | In progress | 验证/重发24h单次、验证事实及邮件/事件已交付；公开发帖/回复的实际调用检查仍依赖C02/B06，不能整条Implemented。 |
| AUTH-008 | Implemented | 三当前政策版本由受信同事务读取/核对并明确同意；测试只用合成版本，正式法律文本与发布留MOD-010。 |
| AUTH-010 | Implemented | 登录/恢复及限流失败固定Argon2工作量；私有账号桶与公开soft挑战响应解耦，过期proof同形拒绝，受限请求不写账号/会话/邮件且不消费proof。 |
| INVITE-001 | Implemented | 随机哈希单次码、active+verified资格、默认3/可选1–7天、过期/撤销及服务端检查，邮箱验证不授T级。 |
| INVITE-002 | Implemented | 创建/消费主体、时间、来源/状态及邀请事实真实持久；用户投影只看本人记录。 |
| INVITE-003 | Implemented | SSR展示UTC今日额度/未用库存/记录、一次性原码及撤销；不返回受邀邮箱/IP/设备。 |
| INVITE-004 | In progress | 仅原模型/来源/受限服务；管理员批量操作、冻结查询与管理UI仍D04。 |
| INVITE-005 | Implemented | no-store/noindex/no-referrer无OG，日志/Outbox/分析/风险仅引用或用途隔离摘要。 |
| INVITE-006 | In progress | UTC累计5与有效未用5、撤销/过期/消费库存释放完整；按信任等级调整额度仍未实现，保留精确缺口。 |
| AUTH-004 | Implemented | 登录/退出/恢复/重置、普通会话/CSRF/重新认证/撤销及密码/邮箱安全通知经过实际本地Worker。异常/新设备通知另属P1 AUTH-104，不靠模板宣称已实现。 |
| AUTH-005 | Implemented | 8–256、可配置下限、版本化弱密码列表及完整性校验离线筛查；启用离线语料后，文件缺失/损坏拒绝Web身份服务启动。按README §6外部依赖交付规则裁决，真实语料来源/更新仍DEC-011，不声称合成语料是完整泄露库。 |
| AUTH-006 | Implemented | DEC-051四维令牌桶与/24、/64真实装配；关闭明确无保护，开启依赖故障失败关闭，私有账号别名与固定成本回归已接。真实Redis矩阵留硬化。 |
| AUTH-007 | Implemented | 双soft桶风险触发、5分钟绑定/单次、版本key保留、不可用失败关闭、配置隔离及LOCAL TEST模拟adapter。依据README §6/04 B02的adapter交付定义；没有真实真人校验/防机器人效果，生产Flag保持关闭、真实provider仍DEC-011。 |
| USER-001 | In progress | 仅真实基础资料/信任/预设头像；内容列表、角色领域、贡献统计与其他数据未接。 |
| USER-004 | Implemented | 500标量纯文本简介、规范邮箱/新邮箱24h确认、版本/owner/CSRF/重认证及全旧会话撤销、双邮箱通知同事务。 |
| USER-005 | Implemented | 正式4–32 ASCII/字母开头、唯一键、反冒充、30天冷却、90天历史保留与CAS；真实SQLite与规则边界测试覆盖。 |
| USER-007 | Implemented | 设备粗摘要、本人单设备/其他设备退出与owner边界；未知提交受限回执查询不恢复已撤会话权限。 |
| TRUST-001 | In progress | pending/T0/T1快照保持、不自动升级；已验证邀请资格rule1同事务。授级及内容实际调用仍后续。 |
| RISK-001 | In progress | 注册/登录/邀请v1业务事件，注册/登录挑战与challenge事实同事务去重；发帖/回复/反馈未接。 |
| RISK-002 | In progress | 实际身份rule1、封闭动作/原因/版本及冲突去重；其他领域规则尚未接。 |
| RISK-003 | In progress | 受信账号/peer IP/设备/网络/邀请HMAC最小信号与独立soft组合；完整时序/渐进策略仍缺，不做单IP封禁。 |
| RISK-007 | In progress | 请求/响应/日志秘密保护与独立风险key已接；正式披露、完整保留期限仍后续。 |
| RISK-008 | In progress | 依赖故障失败关闭，缺完整身份Secret不装配可写service；其他高风险领域尚未接。 |
| DATA-001 / DATA-006 | In progress | 注册/验证/邀请使用真实事务事件及受信分析排除位已接，持久投影去重/重建复用A06；首次行为/贡献/完整排除依据与看板留后续。 |
| INTEG-005 / INTEG-006 | In progress | 正常Web/内嵌/独立Worker共享keyring/source，实际本地输出/抑制/失败重试/重启/旧key恢复；真实发送域/供应商、偏好/退订/反馈鉴权与运维面仍后续。 |
| MOD-010 | In progress | 当前政策读取/一致校验和合成/生产隔离；正式文本、后台发布审批仍未交付。 |

开发验证最终通过：`./scripts/dev-check.sh` r5，241项通过/0失败/1真实Redis测试忽略，真实临时SQLite迁移0001–0008、74条HTTP冒烟；A00冻结与SQL守卫通过。身份46、Web22（实际宿主3）、trust4、moderation2；独立复核发现的两类限流/挑战枚举、装配缺材料、旧key恢复和反冒充边界均已修复。迁移断言及并行日志捕获的失败与复验保留在证据；全真实依赖与安全/并发/性能/视觉矩阵未执行。

B02按本轮完整adapter交付范围记Implemented；B01因明确需求缺口继续In progress。A02/A06/B04/B06继续In progress。所有业务Flag默认关闭，没有配置或Flag写入绕过、首特权入口、推送、部署或真实发送；不授Verified或门禁通过。剩余用户决策为后续T级资格/授予流程、真实provider与语料、正式政策及数据/密钥生命周期操作授权、DEC-049字符折算/自然周；已批本轮常规参数不再重复询问。

### 5.9 公共阅读与内容发布开发登记（2026-09-05，DEC-052）

产品决策与最终批准：项目所有者（用户）；实现、开发验证、中文本机提交：Codex；独立交叉复核：与实现分离的Codex代理。已完整阅读AGENTS，核对README、01/02/03/05/06/04/10/20/90/91/99；实际HEAD=a934e06，开始工作区干净，无后续提交。先单独提交DEC-052/053登记再实施。仅本机，不推送、不部署、不真实邮件。

| 设计项 | 精确范围、接口及完成条件 |
|---|---|
| C01 | NAV-001..005/007/008适用切片：首页最新回复/最新主题/置顶/精华、版块/标签/详情，A00同结构/类名、真实账号菜单与light/dark/system；只有真实数据和空态。搜索、热门浏览口径、未读/未接统计及NAV-009明确缺口，C01整包不完成。 |
| C02 | TOPIC-001/002/003/005主题/首帖/回复、唯一稳定楼层/永久链接、本人编辑与不可变修订、50回复/页；TOPIC-004本人窗口、008真实计数/最后回复受DEC-053限制；CAT-010/011必要读取模型、CAT-012正交状态机、CAT-013/TOPIC-007受限状态合同、CAT-015统一权限。TOPIC-006引用目标/编辑器引用但通知后续。CAT-014/DEC-017移帖、TOPIC-009/010后续，不把普通永久链接当移帖。 |
| C03/C05借入 | EDIT-011原始HTML按文本显示、严格白名单输出；EDIT-001格式/列表/表格/代码/链接/引用、EDIT-002语言标记、安全展示、EDIT-003站内链接/引用、EDIT-004同服务端预览逐项登记。无远程图片、嵌入、抓取、媒体、附件或完整草稿；search_documents仅权威版本/可见性/审核/删除/目录/标题投影，PG可保留正文检索源，SQLite只标题。搜索引擎/页面/Feed/高亮后续，整包仍部分交付。 |
| 身份/资料/风险/分析 | AUTH-003写事务重读邮箱验证、active账号、有效会话/撤销/版本/限制；TRUST-001及RISK-001/002/003/005/006/007/008内容子范围，等级不自动产生。USER-001公开主题/回复列表、USER-002我的主题/回复真实分页并按访问者过滤；角色/贡献/收藏/通知等保留。MOD-009仅发布严格额度切片。DATA-001/003/006和A06真实TopicPublished/ReplyPublished事件，首次/有效贡献/浏览决策未定不产生。 |
| 依赖/交付 | DEC-052技术切片+A00/A01/A03/A04/A05；先合同/迁移/repo/core，再公共SSR/编辑器和普通身份受信adapter，最后风险/分析/Outbox/缓存/观测；本机操作、合成夹具、恢复说明与开发证据。内容参数依DEC-053，未决使用显式未配置合同，测试政策不得冒称批准值。 |
| 数据/迁移/ACL | 仅新PG/SQLite配对迁移0009起，受信表注册及最小ACL；forums扩展、tags/aliases、topics/posts/post_revisions、topic_tags、排序历史、严格发布消费事实、search_documents及必要风险事件类型扩展。首帖与回复同posts，主题专用next_floor计数，(topic_id,floor_no)唯一、FK RESTRICT。修订INSERT/SELECT、runtime无修改删除权；不改0001–0008、不建MySQL/IAM/授级/管理UI/保留清理入口。 |
| 接口/权限 | GET /、/new、/pinned、/featured、/forums/{slug}、/tags/{slug}、/topics/{id}及/posts/{id}永久定位；/topics/new、主题/回复POST、owner编辑/删除、预览及受限幂等结果查询；既有/u/{public_ref}资料及其主题/回复集合、/me/topics、/me/replies。HTTP只解析/CSRF/服务调用；公开读与表单均Flag门控，旧URL/旧表单重读当前权限。未验证仅公开读；待审在本轮普通前台仅owner可见（审核员路径后续），遮蔽/隐藏/删除不在公共投影；会员权限及版块/作者/主题最严格交集覆盖列表、详情、引用、资料和搜索文档。 |
| 锁序/事务 | 幂等→identity账号/会话→trust快照/限制→community提交序列根→版块→主题/帖子，同表按数值ID；所有可变锁完成后追加risk/analytics/Outbox。content受信trait由apps adapter调用identity core及合法trust core，均借当前连接，不访问他域repo。内容、初始/编辑修订、稳定楼层、必要计数/配额、权威搜索文档、幂等结果和Outbox共同提交；版本CAS、唯一约束、回滚和未知提交受限查询沿用A04。 |
| 游标/缓存 | 签名keyset绑定视图/版块/标签/owner/访问主体及固定水位。回复按floor升序、固定已提交水位，新增回复留下一轮；列表按水位时不可变排序历史，避免旧主题被新回复移动造成漏项。提交序列根锁序列化内容变更，防PG自增ID提交重排，历史不自行清理。每次分页仍执行当前可见性，权限收紧即时移除，安全移除不算漏项。缓存命中前主库重读可见性/版本；沿A05公开版本key/30秒TTL及可靠失效Outbox、传播≤60秒，不缓存私密响应。 |
| Flag/故障 | 所有14项业务Flag全关；隔离数据库才受控开public_read/topic_creation/reply_creation等，没有set/配置绕过或AUTH-014之外特权入口。Redis关闭明确NoProtection；启用不可用内容高风险写失败关闭，公开读回库。Strict daily quota属主库业务事实，与可丢失令牌桶分开。未批策略失败关闭。 |
| 开发测试 | 注册/验证真实身份接内容及未验证/未获资格/locked/revoked拒绝；幂等/异载荷/回滚/未知/CAS/唯一；楼层稳定删不重排、50页水位插入不重复漏项；owner/窗口/修订/引用；公开/会员/待审/隐藏/锁定/删除/旧请求；搜索文档共同提交及权限收紧/缓存；Markdown危险URL/事件属性/HTML/代码/引用/预览XSS；配额/事件去重/排除/Worker恢复/日志最小化/Flag与Redis故障；旧迁移/Flag/Worker/A00/A01/A05/B02回归。 |
| 复核/完成 | 独立代理交叉复核授权、事务、可见性、分页、Markdown与日志并修复；适用测试、真实临时SQLite、./scripts/dev-check.sh、A00冻结、git diff --check及逐文件显式暂存后中文本机提交。只有整条完成才Implemented；AUTH-003实际内容检查交付才推进，B01保留INVITE-006，B02维持既有Implemented且不授Verified。C01/C02/C03/C05/B04/B06/A06整包不完成。真实PG/Redis、安全/并发/性能/正式视觉留E01/E02/E04。 |

决策登记前基线 ./scripts/dev-check.sh 已通过（241通过、1真实Redis忽略，临时SQLite0001–0008与74条HTTP）；独立架构/政策复核完成。此为未改代码的基线检查，不是本轮内容实现证据。

本节登记时产品参数待答复；2026-09-06 已由DEC-054/055/056批准，最新状态见第5.10节。最近浏览（NAV-009）保存数量与清理期限仍未定。实现与开发证据将在本节补齐，不以登记、目录或骨架作为完成证据。

2026-09-06独立复核补充依赖：01 §13要求版块/作者/主题最严格权限，PRIV-001/002作者默认值与权威设置未确定。已追加询问并登记DEC-053；2026-09-06 由DEC-054批准为默认公开且用户可关（见第5.10节），默认值与设置入口尚未装配，当前仍按未配置关闭投影，不能将该合同称作者隐私功能已完成。公共空态、当前身份权限重读、受信注入合同、安全渲染与数据库事务继续。

#### 5.9.1 实际交付与逐需求裁决

实现版本以本节关联的中文内容实现提交为准；决策提交为 `cda11ca`、作者隐私补充登记 `b8b131c`。代码在 `crates/content/`、`apps/web/src/content_*`、identity/trust/moderation受限core、PG/SQLite0009/0010与7份新增content模板；原A00模板/资产不修改。本机操作、合成夹具与恢复说明见 [`docs/development/content.md`](../docs/development/content.md)。

| 需求 | 本轮状态 | 已交付与明确剩余范围 |
|---|---|---|
| NAV-001 | In progress | 草莓论坛品牌、核心版块、真实账号菜单及主题模式；搜索和语言入口、正式视觉矩阵仍缺。 |
| NAV-002 | In progress | 四视图真实查询与SSR；正常作者隐私待决，涉及不同作者隐私的最新回复排序口径仍需补批准后的矩阵，不能由隐藏回复推导公开统计。 |
| NAV-003 | In progress | 真实标题/作者/版块/时间和已有组件；浏览数、有效回复数及最后回复口径未批准时显示缺项，不输出0冒充观测。 |
| NAV-004 | In progress | 真实登录状态与最新主题侧栏；热门、最近浏览、站点统计未接，不保留演示数。 |
| NAV-005 | In progress | 首页/版块/标签与本人内容使用绑定主体/筛选/固定已提交水位的签名游标；搜索后续。 |
| NAV-007 | Implemented | light/dark/system服务端链接与继承A00主题样式，SSR转义/主题测试；正式视觉与浏览器矩阵留E04。 |
| NAV-008 | In progress | 同一响应式外壳的真实发布/账号入口；搜索及完整移动端矩阵后续。 |
| NAV-006/009、TOPIC-009/010 | Not started（本切片） | 未读、私有最近浏览、亮回复与跨设备阅读位置留C04，不写未批保存期限。 |
| TOPIC-001/002/003 | In progress | 主题/首帖/回复、本人编辑、永久链接、不可变初始/编辑修订和最后编辑；内容正常政策/作者默认尚未批，故不整条完成。 |
| TOPIC-004 | In progress | 显式窗口、owner/CAS、删除墓碑与修订事务；首帖删除策略必须单独配置，HideTopic只为合成fixture，不默认批准；管理操作后续。 |
| TOPIC-005 | Implemented | 50回复/页、永久floor唯一与next_floor分配、删不重排、签名楼层定位；SQLite覆盖53回复及遍历中插入，PG并发留硬化。 |
| TOPIC-006 | In progress | 引用目标与编辑器、当前目标权限和链接降级；不复制目标正文，未接引用通知。 |
| TOPIC-007、CAT-013 | In progress | 置顶/全局置顶/精华/锁定/隐藏/删除读取合同，本人删除受限；不开放管理写入口、恢复/移动/UI。 |
| TOPIC-008 | In progress | 事务内结构计数/最后回复事实可验证；浏览、有效回复、最后回复与首次行为口径已由DEC-054批准（SSR 200触发、30分钟主体去重、非作者且≥2标量、含作者本人的最新未删回复），正式展示策略与统计装配尚未实现。 |
| CAT-010 | In progress | 多级版块说明/图标/排序/可见性模型与公开读取，已发布独立版规经moderation core同连接注入；管理配置能力后续。 |
| CAT-011 | In progress | 标签、别名、合并目标与受限标签模型/读取/发布校验；管理维护/合并操作后续。 |
| CAT-012 | In progress | 正交字段与本人状态转换闭集；本人编辑不得改审核/发布/讨论/归档轴，删除只设墓碑，其他管理转换未开放；完整管理迁移表后续。 |
| CAT-014/DEC-056 | In progress | DEC-056已批准同对象换版块、主题ID不变、权威永久链接`/t/{id}`不含版块段、历史URL 301；决策阻塞解除，管理移动写入口、301路由与审计接线未实现，不构成完成。 |
| CAT-015 | In progress | 本轮列表/详情/引用/资料/权威搜索统一当前可见性；作者默认可见性已由DEC-054批准为公开但尚未装配，当前作者未知时仍关闭；Feed/sitemap/分享摘要和完整隐私变更传播后续。 |
| EDIT-011 | Implemented | 原始HTML转文字、CommonMark解析及Ammonia严格白名单、危险URL/事件属性/嵌入清理、安全类型与Debug脱敏，10项Markdown测试。 |
| EDIT-001 | In progress | 标题、强调、删除线、引用、列表/任务项、表格、代码块及安全链接；图片不加载，完整工具栏/编辑器增强后续。 |
| EDIT-002/003/004 | In progress | 安全代码语言标记、站内主题/帖子/楼层链接、引用与同服务端预览；语法高亮、提及/自动链接、完整编辑器增强后续，未开放抓取/嵌入。 |
| AUTH-003 | Implemented | 实际内容事务锁后重读验证邮箱、账号状态、会话撤销/凭证版本、CSRF及适用限制；普通注册未验证/已验证pending、锁定/撤销拒绝测试。本轮实现中验证不授T0/T1；DEC-054已批准验证自动授T0，授级装配属TRUST-001剩余工作。 |
| USER-001/002 | In progress | 已有资料与公开/我的主题回复真实集合接线，按当前访问者过滤；DEC-054已批准PRIV-001/002默认公开且用户可关，默认值与设置入口尚未装配，当前仍按未配置关闭投影；贡献/角色/关系/其他集合未接。 |
| TRUST-001、MOD-009 | In progress | 当前信任快照及严格主库消费事实/每日与首24h额度合同；DEC-054已批准邮箱验证自动授T0、T1确定性规则（2自然日+5条有效贡献+无处罚）、T1每日20/100、UTC分日与成功/待审计入且删除不返还，自动授级与正式额度装配尚未实现。 |
| RISK-001/002/003/005/006/007/008 | In progress | 真实内容成功事件与规则版本、可信账号/peer/设备/网络HMAC、actor/action/version二次绑定、四维限流与故障合同、日志脱敏；正常策略未配置，前置拒绝未全量落事件、重复文本/挑战/渐进处置/生命周期与正式阈值未交付。 |
| DATA-001/003/006（A06借入） | In progress | TopicPublished资源为topic.id，ReplyPublished为post.id；同事务Outbox/分析fact、持久去重、账号与合成测试排除及Worker恢复；首次阅读/回复/有效贡献/浏览和完整角色/机器人/活动依据未接，不生成冒名事件。 |
| C05借入search_documents | Implemented（仅同步投影切片） | 内容/可见性/审核/删除/版本与投影共同提交；SQLite不保存检索正文，PG保留源，搜索引擎/页/高亮/Feed/SEO均后续。 |

全部业务Flag默认关闭，仅隔离测试库启用；B02维持既有Implemented并回归，B01保留INVITE-006等缺口。C01/C02/C03/C05、B04/B06/A06继续In progress，任何项目均不授Verified、Released或阶段退出门禁。0009/0010无授级/管理/Flag写口，普通Web没有新特权入口。

开发验证证据：[`validation.json`](../docs/evidence/mvp/content/dev-20260906/validation.json)。`./scripts/dev-check.sh` r5全部通过：281项测试通过、0失败、1项既有真实Redis opt-in测试忽略；真实临时SQLite迁移0001–0010，89条HTTP冒烟，格式化、Clippy全目标`-D warnings`、SQL守卫与diff检查通过。内容26项（core12/Markdown10/HTTP4）、Web32项（内容真实身份/SSR/风险相关10项）、identity48项（新增受信内容接口2项）。A00原件冻结检查通过，PG/SQLite0001–0008逐字节与a934e06一致；只有0009/0010新增。源码SHA-256及已脱敏失败/复验日志一起登记。

独立复核由架构/内容、SSR/风险与Markdown代理交叉进行；已修复旧请求CSRF/限制绕过、引用目标隐去造成来源/投影不一致、作者批量上限、Debug正文、日界线溢出、归档轴误耦合和数据库重试分类。统一验证r1/r2/r3分别暴露旧迁移数量断言、旧骨架首页断言及升级迁移断言，已更新新增行为且保留旧校验/数据保护，r4通过。暂存独立复核又发现SQLite两处globally_pinned约束误引用pinned，已修复并在既有状态测试增加非法整数拒绝断言；r5再次完整通过。真实Unknown丢ACK故障仍复用A04已有验证；本轮实测原操作actor/key查询，不冒称执行真实网络丢ACK。风险四维使用内存/故障适配器验证关闭/开启故障合同，无真实Redis防护效果结论。

真实PG/Redis、安全/并发/性能、正式E2E及视觉矩阵未执行，统一硬化仍待办。首次阅读/回复、浏览和有效贡献不生成完成事件；完整作者隐私、最新回复按不同作者可见性的排序口径、风险前置拒绝事件和其他分析排除依据仍缺。DEC-053已于2026-09-06由DEC-054/055批准（见第5.10节），正常发布政策取得授权值但尚未装配，本轮登记不因此改变实现状态；保留C01/C02/C03/C05、B04/B06/A06整包In progress、B01未处理缺口及全部门禁状态。

### 5.10 产品参数与结构性决策批准登记（2026-09-06，DEC-054..059）

产品决策与最终批准：项目所有者（用户）；本节只登记决策落地与派生同步，不含任何实现交付，不授予任何需求或工作包新的实现状态。

本轮就仓库全部 `Pending` 且影响路线的事项逐项询问并获批 12 项取值，登记为 `DEC-054`（内容产品参数，取代 `DEC-053`）、`DEC-055`（Unicode 标量计量与 ISO-8601 周一起点，补齐 `DEC-049`）、`DEC-056`（移帖跳转，关闭 `CR-012`／`DEC-017`）、`DEC-057`（视觉基准冻结，关闭 `DEC-019`）、`DEC-058`（权限 registry 与 SiteOwner 权限集，关闭 `CR-011`／`CR-013`／`DEC-016`／`DEC-018`）、`DEC-059`（外部 provider 托管服务方向，部分批准 `DEC-011`）。

已按 `README` 第 8 节顺序同步：`06` 决策登记、待决表、冲突段与风险表；`01` §1.3、§3.2、`CAT-014`、`TOPIC-001/004/008`、`NAV-003`、`PRIV-001/002`、`DATA-001/002/003`、`TRUST-001`、`MOD-009`；`02` §8、§10、§15、§17；`03` 第 3 节；`05` 第 2、14 节；`04` 依赖表与 `WP-M0-02`、`WP-MVP-A03`、A06/C 段说明；`10` 第 3 节与退出门禁；`20` 波次说明；`90` 第 6 节固定指标表；本文件第 4.1、5.9、6 节及逐需求备注。

状态影响仅限解除决策阻塞：`CAT-014` 由 `Blocked` 转 `In progress`（管理移动写入口、301 路由与审计未实现），`WP-M0-02` 由 `Blocked` 转 `In progress`（剩余基准冻结记录）。其余需求与工作包状态不变——批准取值不等于已装配，`TRUST-001`／`MOD-009` 的自动授级与 T1 额度、`TOPIC-008` 的浏览与有效回复统计、`PRIV-001/002` 的默认值与设置入口、权限 registry 与 SiteOwner 种子、`visual-baseline-v1` 冻结记录均尚未实现，继续 `In progress` 或 `Not started`。

仍为 `Pending`：`DEC-011` 具体供应商与隐私评估（方向已定）、`DEC-002`、`DEC-003-B`、`DEC-004-B`、`DEC-006-B`、`DEC-007-B`、`DEC-008-B`、`DEC-010-B`、`DEC-012`、`DEC-013`、`DEC-014`，以及 `NAV-009` 最近浏览的保存数量与清理期限。14 项业务 Feature Flag 仍默认全关，不新增写入口或运行时绕过。本轮为纯文档决策提交，未改代码、迁移或配置，不推送、不部署；`Verified` 及以上状态继续只在统一硬化阶段凭证据授予（`DEC-020`）。

### 5.11 正常内容政策装配与信任自动授级（2026-09-06，DEC-054/055，实施前登记）

产品决策与最终批准：项目所有者（用户）；实现、开发期验证与本机中文提交：Claude。实施前工作区仅含未跟踪的 `.workbuddy/` 与 `CLAUDE.md` 符号链接，HEAD=`b0716d3`（决策登记提交）。本节为本轮波次一的实施前登记，不预先宣称任何交付。

#### 精确范围与需求 ID

本轮只做 `DEC-054`／`DEC-055` 批准值可直接解锁、且不依赖新领域能力的装配切片：

| 工作包 | 本轮触及的需求 |
|---|---|
| `WP-MVP-B06` | `TRUST-001`、`MOD-009` 的正常发布策略与额度装配 |
| `WP-MVP-B01` | `AUTH-003` 邮箱验证同事务授予 T0 |
| `WP-MVP-C02` | `TOPIC-001` 长度与标签上限、`TOPIC-004` 30 分钟本人窗口 |
| `WP-MVP-C01` | 作者默认可见性接通后的公共 SSR 投影 |
| `WP-MVP-B04` | `PRIV-001`／`PRIV-002` 默认公开对 `USER-001/002` 列表投影的影响 |

明确不在本轮：`TOPIC-008` 浏览量与有效回复的统计装配、首帖删除占位呈现、`CAT-014` 移帖写入口与 301 路由、权限 registry 与 `/manage`、`visual-baseline-v1` 冻结记录、`NAV-009`。这些留后续波次，本轮结束时相应需求不得因本切片标记完成。

#### 依赖与接口

依赖已交付的 A02 配置、A03 迁移、A04 事务/幂等/Outbox、A05 观测与限流、B01/B02 身份与会话、B06 风险合同、C01/C02/C03/C05 既有内容切片，不等待任一整包。新增接口：

- `content::policy::ApprovedContentPolicy`：版本化常量承载 `DEC-054`／`DEC-055` 批准值，`rules()` 返回经 `ContentRules::validate` 校验的规则。批准值是代码常量而非运行时配置，不新增配置旁路或 Flag 写口。
- `trust::core::grant_verified_tier`：邮箱验证同事务把 `pending` 提升为 `t0`，写 `reason_code` 与规则版本；沿用既有 `identity_snapshots` 根锁与版本防覆盖。
- `trust::core::evaluate_tier`：确定性 T1 规则，由调用方注入「是否存在生效处罚」，账号年龄与有效贡献计数取自 trust 自有事实。规则、版本与原因码位于 trust core，可解释、可重算、可审计（`TRUST-002`）。
- `trust::core::record_contribution` / `withdraw_contribution`：内容发布与本人删除在同一事务内追加或撤销一条有效贡献事实。content 仍不反向依赖 trust，调用经 apps 的 `ContentAuthority` 受信装配注入。

「账号存在满 2 个自然日（UTC）」按自然日读取实现为 `now >= date(created_at) + 2 天的 00:00 UTC`，与 `DEC-048`／`DEC-054` 的 UTC 零点分日一致，也对应 `01` §3.2「跨天活跃」。该读取写入代码注释与本节，不另行扩大解释。

有效贡献计数按 `DEC-054`：主题正文 ≥10 标量、回复 ≥2 标量且回复者非主题作者，且发布状态为 `published`。以 post 唯一的追加事实行记录，本人在窗口内删除时同事务置为不计数，计数即当前有效行数，可从事实行重算。

#### 数据、权限、事务与 Outbox

新增配对迁移 `0011`（postgres/sqlite，mysql8 占位）：`trust.contribution_facts`，含 `user_id`、`kind`、`post_id` 唯一、`body_chars`、`counted`、`created_at`、`version`。同一迁移内写显式 ACL，只授 `app_rw` 必要权限，`worker_rw`／`report_ro` 与 PUBLIC 一律 REVOKE；同时在受信表注册表登记。不改 `0001`–`0010`。

锁序沿用 `DEC-052`：幂等、`identity.users`、`identity.sessions`、`trust.identity_snapshots`、`community.commit_state`、forums、topics、posts，贡献事实与授级在 `trust.identity_snapshots` 锁下完成，风险与分析追加仍在全部可变锁之后。不新增 Outbox 事件类型，不新增 Feature Flag，14 项业务 Flag 继续默认全关。

#### 测试设计

开发期最小集内新增：批准值与 `ContentRules::validate` 一致性；T0 授予幂等与版本防覆盖；T1 规则四类边界（差 1 分钟未满 2 自然日、恰好满、贡献 4 条与 5 条、存在生效处罚）；贡献事实的长度与非作者判定、删除撤销与重算一致；额度 UTC 日界线、待审计入、删除不返还；作者默认公开后的公共 SSR 与未登录访客投影。真实 PostgreSQL/Redis、并发、性能与视觉仍留统一硬化阶段（`DEC-020`）。

#### 5.11.1 实际交付与逐需求裁决（2026-09-06）

代码版本为本节记录所在的实现提交。实施中发现 `DEC-054` 未覆盖 `DEC-053` 待决行里的「外链门槛/四维预算」，已单独询问并由 `DEC-060` 批准后先行登记提交，再接线。

| 交付项 | 内容 |
|---|---|
| 批准发布策略 | `content::policy::ApprovedContentPolicy` 与 `approved_rules()` 承载 `DEC-054/055/060` 全部取值，构造即校验；`APPROVED_RULES_VERSION` 随发布回执落库。正常装配改用该策略，`UnconfiguredContentPolicy` 保留为失败关闭路径。 |
| T0 自动授级 | `trust::core::grant_verified_tier` 在 `mark_email_verified` 同事务把 `pending` 升为 `t0`，幂等、不降级；已验证但仍 pending 的历史行同样收敛。原因码 `email_verified_tier_t0` 加入封闭白名单。 |
| T1 确定性规则 | `trust::core::evaluate_tier`，规则与 `TIER_RULE_VERSION` 位于 trust；「满 2 个自然日」实现为 `now >= 注册日 UTC 零点 + 2 天`。跨域事实（是否存在生效处罚）由 apps 注入，trust 不读他域仓储。只升不降。 |
| 有效贡献事实 | 配对迁移 `0011` 新增 `trust.contribution_facts`（PG 含显式 ACL，只授 `app_rw`；SQLite 同字段；mysql8 占位）。受信表与索引已登记。`record_contribution`／`withdraw_contribution` 由内容写事务在追加段调用，不取新锁；`post_id` 唯一保证重放不重复计数，撤销只置标志位保留可重算历史。 |
| 内容领域接线 | `ContentAuthority` 新增 `record_contribution`；content 在追加段按 `DEC-054` 判定（主题达最低长度、回复达最低长度且发布者非主题作者），删除时产生撤销。content 仍不反向依赖 trust，跨域调用只在 apps 装配层。 |
| 作者默认可见性 | `ApplicationContentAuthority::approved` 把 `author_default` 置为 `Public`（`DEC-054` 批准的 `PRIV-001/002` 默认值），公共 SSR 与作者投影不再整体 `PolicyPending`。 |
| 内容反滥用预算 | `ContentRatePolicy::approved()` 承载 `DEC-060` 四维预算，`APPROVED_RATE_VERSION` 参与受信上下文绑定与写事务二次校验；Redis 关闭时仍明确 `NoProtection`。 |

逐需求裁决（全部维持 `In progress`，本轮不授予 `Implemented`）：

| 需求 | 状态 | 本轮结论 |
|---|---|---|
| `TRUST-001` | In progress | T0/T1 自动授级已按批准规则实现且可复算；等级展示、人工复核与覆盖入口、降级路径未接。 |
| `MOD-009` | In progress | T0/T1 额度、UTC 分日、成功与待审计入、删除不返还、24 小时窗口取严、外链门槛与四维预算均已装配；重复文本与附件门槛未接。 |
| `AUTH-003` | In progress | 验证同事务授 T0 已接；`AUTH-014` 首账号与特权引导仍留后续。 |
| `TOPIC-001`／`TOPIC-004` | In progress | 长度、标签上限、30 分钟本人窗口与首帖删除转主题删除态已按批准值生效；首帖占位呈现属后续波次。 |
| `PRIV-001`／`PRIV-002` | In progress | 账号级默认公开已生效；逐用户隐私设置入口未建，用户当前无法关闭，属明确缺口。 |

测试与证据：`./scripts/dev-check.sh` 七步全过（格式化、Clippy `-D warnings`、287 项单元测试全通过、真实 SQLite 冒烟 88 条路由响应、`git diff --check`、暂存区复核、静态守卫）。本轮新增测试覆盖批准常量与决策记录逐项一致、Unicode 标量计量与标题闭区间边界、外链门槛分级、T1 规则四类边界（贡献差一条／差一分钟未满两自然日／存在生效处罚／三者同时满足）、授级幂等与不降级、贡献事实幂等与撤销后可重算、内容侧有效贡献判定（自回复不计入、他人回复计入、删除撤销）。既有断言中编码 `DEC-051`「不自动授 T0/T1」的 7 处已按 `DEC-054` 更新并注明依据。未执行真实 PostgreSQL/Redis、并发、性能、安全与视觉矩阵。

本轮明确未做（不得因本切片视为完成）：`TOPIC-008` 浏览量与有效回复的统计装配、首帖删除后的占位呈现、`CAT-014` 移帖写入口与 301 路由、逐用户隐私设置入口、权限 registry 与 `/manage`、`visual-baseline-v1` 冻结记录、`NAV-009`。

#### 边界

批准取值不等于整包完成：`C01/C02/C03/C05`、`B01/B04/B06/A06` 整包不因本切片标记 `Implemented`，状态按各自剩余条款裁决。不授予 `Verified` 或任何阶段/发布门禁。仅本机开发、独立复核、开发期最小集与中文新建提交，不推送、不部署、不发送真实邮件，不清空非本地数据库。

### 5.12 隐私设置入口、浏览统计与有效回复（2026-09-06，波次二实施前登记）

产品决策与最终批准：项目所有者（用户）；实现、开发期验证与本机中文提交：Claude。实施前工作区干净，HEAD=`9bd0243`。本节为波次二实施前登记，不预先宣称任何交付。

#### 精确范围与需求 ID

| 工作包 | 本轮触及的需求 |
|---|---|
| `WP-MVP-B04` | `PRIV-001`／`PRIV-002` 逐用户隐私设置入口，补齐波次一留下的缺口 |
| `WP-MVP-C01` | `NAV-003` 主题行浏览数展示、`NAV-004` 热门排序的真实阅读量来源 |
| `WP-MVP-C02` | `TOPIC-008` 有效回复数与最后回复、`TOPIC-004` 首帖删除后的占位呈现 |
| `WP-MVP-A06` | `DATA-001` 首次阅读事件、`DATA-003` 有效回复数的原始业务事实 |

明确不在本轮：`CAT-014` 移帖写入口与 301 路由、权限 registry 与 `/manage`、`visual-baseline-v1` 冻结记录、`NAV-009` 最近浏览、跨设备阅读位置、搜索页与 Feed。

#### 接口与实现路径

- **隐私设置**：`identity.users` 增加 `profile_lists_public`、`topic_body_public` 两个布尔列，默认真（`DEC-054` 批准的默认公开）。新增 `/settings/privacy` 页面与 `SettingsService::update_privacy` 命令，沿用既有 CSRF、幂等键、版本校验与账号根锁；不新增特权入口。`ContentAuthority::authors` 与 `authorize` 改为读取每个作者的真实设置，`author_default` 退化为「设置缺失时的账号级默认」。
- **有效回复数**：`TopicView.reply_count` 改为 `DEC-054` 的有效回复口径——已发布、当前访问者可见、正文 ≥2 标量、且发布者不是主题作者；同一用户在同一主题内可重复计入。最后回复仍取最新一条未删回复（含作者本人），两者语义分列，不互相替代。
- **首帖删除占位**：首帖删除后主题进入删除态，公开投影渲染占位提示而不是直接 404，楼层号与回复记录保留、计数不回退。占位文案走 `05` 已冻结的模板结构与 CSS 类名，不新增视觉资产。
- **浏览量**：新增 `community.topic_views` 去重台账（`topic_id` + `subject_hash` 唯一，记 `last_viewed_at`）与 `community.topics.view_count` 计数列。触发点为主题详情页 SSR 成功返回 200 后的独立短写事务，不进入读事务，也不阻塞渲染；写失败只记观测，不影响响应。
  - 去重主体：登录用户按用户 ID，游客按会话 cookie，无会话时按设备指纹。原始值不落库，一律经用途隔离 HMAC 派生为 `subject_hash`，密钥由应用装配层注入（沿用 `DEC-051` 的用途隔离派生约定），content 不接触原始 IP 或 Cookie。
  - 去重窗口 30 分钟（`DEC-054`）。窗口内重复访问只更新 `last_viewed_at`，不增加计数。
  - 排除：作者本人、`analytics_excluded` 账号（含系统账号与已确认作弊流量）、已知爬虫 UA。爬虫判定使用保守的 UA 子串清单，判定不到就照常计数，不做启发式封禁。
  - `Viewer` 增加 `excluded` 标志承载账号级排除事实，由 `ContentAuthority::read_viewer` 填充。

#### 数据、权限、事务与 Outbox

新增配对迁移 `0012`（postgres/sqlite，mysql8 占位）：`community.topic_views` 表与 `community.topics.view_count` 列，同迁移内写显式 ACL（只授 `app_rw` 必要权限）并登记受信表与索引；`identity.users` 增加两个隐私列。不改 `0001`–`0011`。

浏览计数是独立短事务，锁序为 `community.topics` 单表，不与发布事务交叉。隐私设置沿用身份既有锁序。本轮不新增 Outbox 事件类型、不新增 Feature Flag，14 项业务 Flag 继续默认全关。首次阅读事件按 `DEC-054` 定义为「该用户生涯首次计入浏览」，由浏览事务在同事务内判定并落分析事实。

#### 测试设计

开发期最小集内新增：隐私设置的默认值、关闭后未登录访客投影收敛、本人始终可见；有效回复计数排除作者本人与短回复且与最后回复语义分列；首帖删除后主题占位可读且楼层与计数不回退；浏览去重窗口内外的计数差异、作者本人与被排除账号不计数、爬虫 UA 不计数、原始主体值不落库。真实 PostgreSQL/Redis、并发、性能与视觉仍留统一硬化阶段（`DEC-020`）。

#### 边界

本轮结束后按剩余条款逐需求裁决，不因切片把整包标记完成，不授予 `Verified` 或任何阶段门禁。仅本机开发、独立复核、开发期最小集与中文新建提交，不推送、不部署、不发送真实邮件。新增与改动代码一律使用详细中文注释。

## 6. 门禁状态

| 门禁 | 当前状态 | 原因 |
|---|---|---|
| M0 Repository Ready | In progress | `WP-M0-01`、`WP-M0-03`、`WP-M0-04`、`WP-M0-05` 已为 `Implemented`；`WP-M0-02` 的 `DEC-019` 已由 `DEC-057` 关闭，转 `In progress`，剩余基准冻结记录；`WP-M0-06` 尚未完成，统一硬化证据和阶段门禁尚未通过。 |
| M0 Production Approved | Blocked | 多项决策、人员和生产事项待定 |
| MVP Repository Ready | In progress | `WP-MVP-A00/A01/A03/A04/A05/B02` 已完成开发期交付，`A02/A06/B01/B04/B06` 仍为部分交付，其余 MVP 工作包及统一硬化尚未完成；DEC-045/046本机版本化不代替正式门禁。 |
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
