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
| `WP-M0-04..06` | Not started | 已存在的架构和配置骨架属于提前开发，不替代这些工作包所需的负责人登记、完整交付与评审证据。 |
| `WP-MVP-A00` | Implemented（开发期重做，2026-09-05） | 14个隔离SSR夹具、30项本地源资产、模板结构/CSS类名冻结、17张本机原始参考截图和开发期检查已交付，详见第5.1节。按用户修改后的 `05` 与 `DEC-045` 如实登记未知许可并本机版本化；E04/DEC-019仍未执行，不授予 `Verified`。代码版本为本条记录所在实现提交。 |
| `WP-MVP-A01` | In progress（开发期，2026-09-05） | 交付物随 `386df32` 与后续评审提交落地：`Cargo.toml`（16 成员 workspace，`rust-version = 1.88`）、三个可执行、`rustfmt.toml`、`rust-toolchain.toml`（钉 1.97.0）、七步 `scripts/dev-check.sh` 与 `smoke-routes.txt`、`LICENSE`、`.editorconfig`、`.gitattributes`。本轮 M0-03 共用交付 Node/npm/esbuild 和 Cargo/npm 锁定策略、Syft/Trivy/Gitleaks 配置与扫描入口，见第 5.2 节；不再另建 cargo-deny/cargo-audit 或另一套 npm 锁文件。**待交付**：实际 esbuild 管线、static/dist 内容哈希、manifest helper 与 immutable 资产路由。正式全量依赖/Secret 扫描、跨平台锁定构建留 E01。当前最小集 65 个单元测试和 48 条 HTTP 冒烟通过，SQLite 迁移执行器未接入而跳过（A03），不记作迁移验证。依赖仍按 `04` 的 M0-01/M0-03，开发前置交付不代表 A01 完成。 |
| `WP-MVP-A02` | In progress（开发期，2026-09-05） | 部分交付：`config/default.toml` 与 development/test/production 三份脱敏模板（含 `server.environment`、`database.schema_prefix`、`max_connections`）、`crates/platform/src/config.rs`（`Environment` 枚举、全结构 `deny_unknown_fields`、`validate()` 覆盖预留后端拒绝启动／SQLite 单写者／前缀合法性／绑定地址／生产必须启用 Redis）、`crates/platform/src/flags.rs`（14 个 MVP flag，默认全关，key 唯一性有单元测试）。**待交付**：配置分层加载与 `config/local.toml` 覆盖、flag 持久化与两个受控写入口（`DEC-028`）、只读与紧急关闭 |
| `WP-MVP-A03` | In progress（开发期，2026-09-05） | 部分交付：`db/migrations/{postgres,sqlite,mysql8}/` 三轨目录、`crates/platform-db/src/migrations.rs` 的 `verify_alignment` N 轨编号对齐骨架（文件名格式、同轨重复编号、集合差异与仓库目录测试）、`crates/platform-db/src/lib.rs` 双后端连接池（连接数取自配置、SQLite WAL/busy_timeout/外键、FTS5 单连接探测、PostgreSQL 扩展探测）、`crates/platform/src/identifiers.rs` 的受信 `SchemaRenderer`（15 个逻辑 schema 注册表、表名校验、63 字节上限、`DEC-029` 渲染）。**已知缺口**：当前 `verify_alignment` 豁免所有空轨，尚未收紧为只豁免 `mysql8`；PostgreSQL 或 SQLite 单轨为空时可能误判通过。**待交付**：修复该缺口、迁移执行器、初始 schema、`bbbcm-cli db migrate/status/seed`、PostgreSQL 角色与 ACL policy、共享 SQL lint。 |
| `WP-MVP-A04..A06` | Not started | - |
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

## 6. 门禁状态

| 门禁 | 当前状态 | 原因 |
|---|---|---|
| M0 Repository Ready | In progress | `WP-M0-01`、`WP-M0-03` 已为 `Implemented`；`WP-M0-02` 受 `DEC-019` 阻塞，`WP-M0-04..06` 尚未完成，统一硬化证据和阶段门禁尚未通过。 |
| M0 Production Approved | Blocked | 多项决策、人员和生产事项待定 |
| MVP Repository Ready | In progress | `WP-MVP-A01..A03` 已部分落地，`WP-MVP-A00` 已完成本轮开发交付（DEC-045本机版本化），其余 MVP 工作包及统一硬化尚未完成。 |
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
