# Rust 骨架设计稿（2026-09，历史存档）

> **非事实源。** 本文是 2026-09-04 会话期间的设计工作稿，`DEC-026` 已将其退役并移入本目录，仅供追溯设计取舍的由来。
>
> - 唯一计划入口与事实源是 [`plan/README.md`](../../plan/README.md)；技术约束以 [`plan/02-technical-architecture.md`](../../plan/02-technical-architecture.md) 为准。
> - 本文与 `plan/` 冲突时**一律以 `plan/` 为准**，不得据本文实施。
> - 本文原第 5 节的实施进度与验证结果已迁入 [`plan/99-traceability.md`](../../plan/99-traceability.md) 第 5 节；此处不再保留任何状态登记。
> - 本文成文时的若干表述此后已被修订：限流改由 Redis 承载（`DEC-024`，本文第 3 节的“进程内令牌桶”口径作废）；可执行程序名与前缀改名已补登为 `DEC-025`；MySQL 目标版本及下文全部 `mysql57` 口径已由 `DEC-033` 修订为 MySQL 8.0/`mysql8`；SQLite `BEGIN IMMEDIATE` 与 MySQL `DATETIME(6)` 已并入 `02`。

## 0. 部署定位（新增约束）

本项目定位为开源软件，提供两种部署形态：

| 形态 | 数据库 | Redis | 目标 |
|---|---|---|---|
| 完整模式 | PostgreSQL 16 | 可选开启 | 本项目正式部署与中大型社区 |
| 体验模式 | SQLite 单文件 | 不需要 | 开源用户零依赖快速部署体验 |

- 体验模式允许功能降级（降级矩阵见 2.5），核心社区功能（注册、发帖、回复、举报、基础后台）必须完整。
- 后端由配置 `database.backend = "postgres" | "sqlite"` 决定，运行时二选一（预留 `"mysql57"` 值，未实现时拒绝启动，见 DEC-022）；明确不提供 SQLite → PostgreSQL 的升级或迁移工具，跨后端不做任何数据迁移承诺。

## 1. Rust 基础框架设计（v3，已确认）

状态：2026-09-04 用户确认。v3 变化：领域按领域拆分为 8 个 crate；模板目录按路由命名（维持 v1 方案）；限流统一进程内实现；FTS5 采用启动探测 + 自动降级；明确不提供 SQLite → PG 迁移。

### 1.1 Workspace 总体布局

Cargo workspace 模块化单体：根 `Cargo.toml` 统一管理依赖版本和 profile，3 个可执行 + 6 个库 crate。

```text
bbbcm/
├── Cargo.toml              # workspace 根：统一依赖、edition、rust-version、profile
├── apps/
│   ├── web/                # 可执行：Axum HTTP + Askama SSR
│   │   ├── src/
│   │   │   ├── main.rs         # 启动、配置加载、依赖装配
│   │   │   ├── app_state.rs    # 数据库连接、缓存/限流实现、Feature Flag、模板上下文
│   │   │   ├── router.rs       # 路由挂载点：只组合领域 router，不写业务
│   │   │   ├── middleware/     # 会话、CSRF、限流、权限版本、请求日志
│   │   │   ├── error.rs        # AppError → HTTP 状态/错误页的映射
│   │   │   ├── assets.rs       # 静态资产指纹 manifest 读取
│   │   │   └── jobs.rs         # worker.embedded=true 时的进程内任务循环
│   │   ├── templates/          # Askama 模板（见 1.3）
│   │   └── static/             # 静态资产源文件（见 1.4）
│   ├── bbbcm-worker/       # 可执行：Outbox 消费、邮件、媒体、分析任务
│   │   └── src/{main.rs, jobs/, sinks/}
│   └── bbbcm-cli/          # 可执行：迁移、种子、flag、运维命令
│       └── src/{main.rs, commands/}
├── crates/
│   ├── platform/           # 配置、SchemaRenderer、Feature Flag、错误、通用类型
│   │   └── src/
│   │       ├── db/         # Backend 抽象、连接管理、SQL 占位符渲染（见 2.1）
│   │       ├── cache/      # CacheStore/Wakeup/Lease trait（Redis 与内存实现）+ 进程内限流 ratelimit
│   │       └── …           # flags.rs、identifiers.rs、types.rs、error.rs
│   ├── platform-db/        # 事务模板与重试、repo 基础 trait、测试支持
│   ├── platform-outbox/    # Outbox 模型、lease、sink trait、重试、DLQ（双后端）
│   ├── platform-audit/     # 管理操作审计日志
│   ├── identity/           # 领域 crate：用户、凭证、会话、邀请、验证、MFA
│   ├── content/            # 领域 crate：版块、主题、回复、草稿、markdown.rs
│   ├── media/              # 领域 crate：附件、扫描、转码
│   ├── interaction/        # 领域 crate：收藏、关注、反馈、阅读位置、通知
│   ├── moderation/         # 领域 crate：举报、案件、处罚、申诉
│   ├── support/            # 领域 crate：客服工单、账号恢复
│   ├── trust/              # 领域 crate：信任等级、积分账本 ledger.rs、风险 risk.rs
│   ├── privacy/            # 领域 crate：数据请求、删除、导出
│   └── manage/             # RBAC/ABAC registry、审批与 /manage 契约
├── config/                 # default.toml + 各环境脱敏模板
├── db/
│   └── migrations/
│       ├── postgres/       # PostgreSQL 迁移集合（0001_init.sql 起）
│       ├── sqlite/         # SQLite 迁移集合（编号与 postgres 对应同一批结构变更）
│       └── mysql57/        # MySQL 5.7 迁移集合（空占位，N 轨见 2.3）
├── scripts/                # 可重复验证脚本
├── tests/                  # 集成与 E2E（双后端矩阵，见 2.6）
│   ├── fixtures/
│   └── e2e/
```

### 1.2 每个 API 请求放哪里

核心规则：`apps/web/src/router.rs` 只做挂载，不写业务；每个领域一个 crate，自带路由和 handler，crate 内部固定四层。

领域 crate 内部结构（以 `content` 为例）：

```text
crates/content/src/
├── contract.rs   # 请求/响应/视图类型、状态枚举、领域错误码
├── core.rs       # 应用服务与状态机：开事务、取锁、调 repo、写 Outbox
├── repo.rs       # 数据访问：只借用当前事务连接，不自行提交；SQL 见 2.1 约定
├── http.rs       # Axum Router + handler：解析输入、鉴权调用、渲染/JSON
│                 # 文件过大时按资源拆 http/ 子目录（topic.rs、reply.rs…）
├── markdown.rs   # 仅 content 有的子模块示例
└── tests.rs      # 领域单元测试
```

请求归属一览：

| 请求类型 | 位置 | 示例 |
|---|---|---|
| 公共页面与表单 | `crates/<领域>/src/http.rs` | 发帖 `POST /t/{id}/reply` → `crates/content/src/http.rs` |
| 登录注册等 | `crates/identity/src/http.rs` | `POST /login`、`POST /register` |
| `/manage` 页面与 `/manage/_api/v1/*` | `crates/manage/src/console/<区域>/http.rs` | 举报队列 → `console/reports.rs` |
| 异步外部副作用 | `apps/bbbcm-worker/src/sinks/`（或内嵌 jobs） | 发邮件、媒体转码 |
| 运维命令 | `apps/bbbcm-cli/src/commands/` | `bbbcm-cli db migrate --backend sqlite` |

一次请求的固定链路：`middleware`（会话/CSRF/限流）→ `http.rs` handler（解析+校验输入）→ `core.rs` 服务（事务+锁+状态机）→ `repo.rs`（借事务连接执行 SQL）→ Outbox/操作日志同事务提交 → Askama 渲染或 JSON。Handler 禁止 import `sqlx`；跨领域读取走对方 core 暴露的只读服务。

领域 crate 之间只允许依赖对方的 `contract` 类型与 core 只读服务，禁止 import 对方 repo 或直接查询对方领域的表。

### 1.3 HTML 模板

- 统一放 `apps/web/templates/`，Askama 编译期检查，模板编译进二进制。
- 组织方式：`layout/base.html` → `partials/`（页头、页脚、分页、表单、错误块）→ 按页面路由分目录：

```text
apps/web/templates/
├── layout/base.html
├── partials/{header,footer,pagination,form_fields,error_block}.html
├── home/index.html
├── forum/{index,topic_list}.html
├── topic/{show,new,edit}.html
├── auth/{login,register,password_reset}.html
├── user/{profile,bookmarks,notifications}.html
├── settings/index.html
└── manage/{login,dashboard,reports,users}.html
```

- 模板只接收 `contract.rs` 的视图类型；错误态和空状态是必填块，缺失在编译期报错。
- 搜索页在 SQLite 模式显示"体验模式：仅支持标题搜索"提示（由视图类型携带 backend 能力标志）。

### 1.4 静态资产

- 源文件 `apps/web/static/{css,js,img,fonts}/`；构建时生成内容哈希文件名到 `static/dist/` + `manifest.json`。
- `assets.rs` 提供模板 helper 按 manifest 解析指纹路径；指纹路径响应 `Cache-Control: immutable, max-age=1y`。TypeScript 很少量，仅增强交互，核心流程无 JS 可用。

### 1.5 其他基础设定

- 依赖方向：`apps/* → crates/*`；8 个领域 crate 与 `manage` 不依赖任何 `apps`；`manage` 可依赖领域 crate 与 `platform-*`；领域 crate 依赖 `platform-*`，相互之间仅限 contract 与 core 只读服务；`platform-*` 除可依赖 `platform` 外不横向依赖。
- 强类型 ID：`platform::types::Id<T>`，JSON 以十进制字符串传输；列表统一签名 keyset 游标。
- 错误模型：`platform::AppError` 唯一出口，`web/src/error.rs` 统一映射 HTTP 状态码和错误页；领域错误码各自定义在 `contract.rs`。
- Schema 渲染：`platform::SchemaRenderer` 唯一生成物理对象名（PG：`bcm_<schema>.<table>`；SQLite：扁平 `<table>`），查询值一律 bind 参数。
- 配置分层：`config/default.toml` 安全默认 → 环境模板 → 环境变量 → 部署密钥注入；关键键：`database.backend`、`database.sqlite.path`、`features.redis`、`worker.embedded`；非法/缺失配置拒绝启动，`.env` 不入库。
- Feature Flag：`crates/platform/src/flags.rs` 唯一注册点，默认全关。
- 命名固定：可执行 `bbbcm-worker`、`bbbcm-cli`；Redis key `bcm:<env>:v1:<domain>:<key>`（仅 Redis 实现内部）；数据库 schema 前缀 `bcm_`（如 `bcm_identity.users`）。
- 质量基线：`rustfmt.toml` 固定格式，Clippy `-D warnings` 本地强制。

### 1.6 `bbbcm-worker` 与 `bbbcm-cli` 的职责

#### bbbcm-worker：后台任务进程

web 进程只做"请求范围内"的事（渲染页面、提交事务）；所有慢的、定时的、对外部系统的活儿交给 worker。核心是一个循环：轮询 Outbox 表 → 租约取走待处理事件 → 分发给对应 sink。

| 分类 | 内容 | 位置 |
|---|---|---|
| Outbox sink | 发邮件（验证、找回密码、通知）、站外 Webhook（后续阶段） | `src/sinks/mail.rs` 等 |
| 媒体处理 | 图片转码、缩略图、元数据清除、隔离扫描后发布 | `src/sinks/media.rs` |
| 定时清理 | 过期会话/草稿/邀请、回收站附件清理、删除墓碑执行 | `src/jobs/cleanup.rs` |
| 聚合任务 | 通知摘要、分析指标汇总、搜索重建调度 | `src/jobs/aggregate.rs` |

行为规则：

- 只消费**已提交**的 Outbox 事件；业务事实永远在 web 的事务里写完，worker 不补写业务数据。
- 每个 sink 以 `(sink, event_id)` 幂等去重，失败指数退避，最终进 DLQ；外部故障（SMTP 挂了）不回滚业务、不伪造成功。
- 事务引擎、租约、重试都实现在 `crates/platform-outbox`；worker 二进制只是宿主进程（装配 + 运行循环），自身不写业务规则。
- PG 模式支持多 worker（`SKIP LOCKED`）；SQLite 模式单写者租约，建议单实例，或 `worker.embedded = true` 直接内嵌进 web 进程。

#### bbbcm-cli：运维与引导命令行

不放 HTTP 的操作都放这里——尤其首次部署时还没有管理员账号、没有 HTTP 可用的场景。

| 命令 | 作用 |
|---|---|
| `bbbcm-cli db migrate --backend <pg\|sqlite>` | 执行对应后端的迁移集合 |
| `bbbcm-cli db status` | 查看迁移与结构版本 |
| `bbbcm-cli db seed` | 初始数据：四个版块、规则、首个管理员 |
| `bbbcm-cli user create / promote / reset-password` | 引导管理员与救援操作 |
| `bbbcm-cli flag list / set` | Feature Flag 受控修改（默认全关） |

规则：cli 同样是薄壳——参数解析（clap）+ 调用 crate 里的服务（迁移逻辑在 `platform-db`，用户创建走 `identity` 的 core），保证同一套逻辑被 web/cli/worker 共用；体验模式（SQLite）也必须经由 cli 完成 migrate + seed 再启动 web。

## 2. SQLite 兼容设计（已确认）

### 2.1 数据库抽象层

- `platform::db::Backend` 枚举：`Postgres | Sqlite`，预留 `Mysql57`（配置 `database.backend` 预留 `"mysql57"` 值，未实现时拒绝启动；MySQL 5.7 规范先行、现在不实现，DEC-022）；`platform-db` 提供 `Conn`（对 `PgConnection`/`SqliteConnection` 的枚举封装）和统一事务模板（BEGIN/COMMIT/ROLLBACK、`40001/40P01` 有界重试仅 PG 生效）。
- SQL 编写约定：repo 中一律写 `?` 占位符，`platform::db::render_sql(backend, sql)` 在 PG 下转换为 `$1..$n`，SQLite 下原样；标识符一律经 `SchemaRenderer`。不使用 `sqlx::query!` 编译期宏（它绑定单一后端）。
- 可移植子集（DEC-022，收紧为 PostgreSQL 16 + SQLite + MySQL 5.7 三库交集）：仅 `SELECT/INSERT/UPDATE/DELETE`、JOIN、子查询、`LIMIT`。禁用 `RETURNING`、CTE、`ON CONFLICT ... DO UPDATE`；upsert 统一为 update→insert 两步，或集中在 `platform::db` 的方言分支函数；主键回读走平台层 `last_insert_id` 抽象。全局禁用触发器、存储过程、视图、部分索引和数据库端 ID 函数。PG 专属语法（`ILIKE`、`FOR UPDATE`、`SKIP LOCKED`、`::cast`、区间类型）不允许直接出现在共享 SQL 中，放入 `match backend` 分支。
- 锁策略：PG 用 `FOR UPDATE` 行锁；SQLite 依赖单写者 + WAL + `busy_timeout`，事务内不需要行锁语法（分支为空实现或 `BEGIN IMMEDIATE`）；MySQL 5.7 行锁与 `SKIP LOCKED` 替代语义集中在方言分支，接入时定。
- 回避方案说明：曾考虑"每个领域两套 repo 实现互不干扰"，工作量和漂移风险都高；采用"共享可移植 SQL + 少量方言分支"路线，方言分支集中在 repo 文件尾部并标注 `// dialect:`。

### 2.2 类型映射

| 概念 | PostgreSQL | SQLite | MySQL 5.7（预留，未实现） |
|---|---|---|---|
| 主键 | `BIGINT IDENTITY` | `INTEGER PRIMARY KEY`（rowid 别名） | `BIGINT AUTO_INCREMENT` |
| 时间 | `timestamptz` | `TEXT`（RFC3339，UTC） | `DATETIME(6)`（UTC，应用层解析） |
| 布尔 | `boolean` | `INTEGER` 0/1 | `TINYINT(1)` 0/1 |
| 枚举/状态 | 文本 + `CHECK` | 文本 + `CHECK` | 文本 + `CHECK`（5.7 静默忽略，仅作文档） |
| JSON 载荷 | `TEXT`（应用层解析校验） | `TEXT`（应用层解析校验） | `TEXT`（应用层解析校验） |
| 事件/操作 ID | UUIDv7 文本 | UUIDv7 文本 | UUIDv7 文本 |
| 字符串唯一键与比较 | 二进制语义，应用层规范化 | `BINARY`（默认），应用层规范化 | `utf8mb4_bin`，应用层规范化 |
| 全文索引 | `pg_jieba` GIN + `pg_trgm` | FTS5（可选）或标题 LIKE | `FULLTEXT` + ngram（接入时定） |
| 迁移元数据 | `bcm_platform.schema_migrations` | `schema_migrations` | `schema_migrations`（接入时定） |

DEC-022 约束：枚举/状态的 `CHECK` 三库均仅作文档（MySQL 5.7 静默忽略），取值校验一律在应用层；JSON 载荷一律 TEXT 存储、应用层解析；布尔统一 0/1；时间统一 UTC。搜索为应用层 `SearchBackend` 抽象（PG `pg_jieba + pg_trgm` / SQLite FTS5 或 LIKE / MySQL 5.7 `FULLTEXT` + ngram 后续），三种全文方言不出现在共享 SQL 中。

读写转换（时间解析、bool 解码、JSON 文本编解码）统一封装在 `platform::db::convert`，repo 不手写。

### 2.3 迁移 N 轨

- `db/migrations/{postgres,sqlite,mysql57}/` 多轨有序文件，同一编号代表同一批结构变更（如各轨都有 `0007_invitations.sql`）；`mysql57` 轨暂为空占位（DEC-022：MySQL 5.7 规范先行，现在不实现）。
- `bbbcm-cli db migrate --backend <postgres|sqlite>` 选择集合执行；元数据表记录已应用版本；`--backend mysql57` 为预留值，未实现时拒绝启动。
- 配对校验泛化为多集合编号对齐（`platform_db::migrations::verify_pairing`）：已实现各轨的编号集合必须一一对应；空占位的 `mysql57` 轨在实现前豁免对齐并显式标注。
- SQLite 连接统一 `PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000; PRAGMA foreign_keys=ON;`。

### 2.4 Outbox 与 worker 行为

- Outbox 表结构两后端同构，业务事件随主库事务提交，语义一致。
- PG worker：`FOR UPDATE SKIP LOCKED` 批量租约，多 worker 安全。
- SQLite worker：单写者租约分两步——先 `UPDATE ... SET lease_owner=?, lease_until=? WHERE id IN (SELECT id FROM ... WHERE 可租约)`，再按 `lease_owner` SELECT 回读本批事件（不使用 `RETURNING`，DEC-022；如改用方言分支实现，须集中在 `platform-outbox`）；建议只跑一个 worker。
- MySQL 5.7（预留）：无 `SKIP LOCKED`，租约与行锁为集中方言分支，接入时定。
- 体验模式 `worker.embedded = true`：`apps/web/src/jobs.rs` 在 web 进程内跑任务循环（tokio interval 轮询 Outbox），实现"单二进制 + 单数据库文件"部署。

### 2.5 功能降级矩阵

| 能力 | PostgreSQL 完整模式 | SQLite 体验模式 |
|---|---|---|
| 搜索范围 | 标题 + 正文，中文分词、模糊匹配 | 仅标题（FTS5 可用时用 FTS5，否则 LIKE） |
| 相关性排序 | jieba 权重 + trgm | 按时间/精确匹配简单排序 |
| 并发写入 | 多 worker、多实例 | 单写者，建议单 worker |
| 容量定位 | 1 万用户 / 100 万帖基线 | 小规模体验（数百用户 / 数万帖量级） |
| 限流 | 进程内令牌桶（与 SQLite 一致） | 进程内令牌桶 |
| 缓存 | Redis 共享缓存（开启时） | 进程内 LRU 或直接回库 |
| 领域功能 | 完整 | 完整（同一套 core/repo 代码） |

搜索实现细节：`search_documents` 投影在 SQLite 模式只维护标题、可见性、状态与游标字段；标题检索优先用 FTS5 external-content 表，不可用时降级为 `normalized_title LIKE ?`（写入时同步维护小写规范化列）。搜索页文案明示降级。

FTS5 探测机制（已确认）：启动时执行一次能力探测——在 temp 库尝试 `CREATE VIRTUAL TABLE temp.fts5_probe USING fts5(x)`，成功即判定 FTS5 可用并随即删除探针表；失败只记录一行日志，不报错、不阻断启动。探测结果缓存进 `platform::db` 的后端能力集（capabilities），运行期不再重复探测；搜索 repo 按能力集选择 FTS5 或 LIKE 分支。

### 2.6 测试策略

- 验证分两层（DEC-020）：
  - 开发期每次提交只执行最小集：格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟、`git diff --check` 与暂存区复核；开发期需求与工作包最高标记 `Implemented`，`Verified` 及以上状态只能在硬化阶段凭证据授予。
  - 统一硬化阶段（MVP 的 E 段工作包）集中执行：真实 PostgreSQL 双前缀集成、真实 Redis 合同、后端矩阵（PostgreSQL 跑全量；SQLite 跑核心流程——注册、登录、发帖、回复、标题搜索、举报、基础后台——+ 降级断言，如正文搜索入口返回受限提示而非错误）、安全套件、并发、性能与容量基线、WCAG、六视口像素差异、证据 manifest、SBOM/secret/许可证扫描。
- `tests/` 支持按 `TEST_BACKEND` 选择后端。
- 迁移一致性测试：N 轨迁移编号集合对齐（脚本校验；空占位轨豁免规则见 2.3）。
- 设计纪律不随验证后置而放松：幂等、事务边界、Outbox 同事务提交、版本防覆盖、Feature Flag 默认全关且后续能力路由服务端不可达、公开仓库脱敏。

## 3. Redis 可选设计（已确认）

- 限流独立于 Redis 和数据库后端：`platform::cache::ratelimit` 提供统一的进程内令牌桶实现，PostgreSQL 与 SQLite 两种模式行为一致，多实例部署时为按实例近似（已在 2.5 降级矩阵注明）。
- `platform::cache` 定义三个 trait：`CacheStore`、`Wakeup`、`Lease`；提供两套实现：`RedisImpl`（多实例）与 `MemoryImpl`（单进程内存）。
- `features.redis = true`：装配 RedisImpl，缓存共享、worker 唤醒低延迟；`false`：装配 MemoryImpl——缓存改进程内 LRU、worker 唤醒改为固定间隔轮询。
- 会话、幂等记录、业务事实始终在主库，Redis 关闭不影响正确性（Redis 从不承载权威数据）。
- 体验模式默认组合：SQLite + `features.redis=false` + `worker.embedded=true`，即单机单文件零依赖部署。

## 4. 决策记录（2026-09-04 用户确认）

1. 领域 crate 按领域拆分为 8 个：`identity`、`content`、`media`、`interaction`、`moderation`、`support`、`trust`、`privacy`；`markdown.rs` 归 `content`，`notification.rs` 归 `interaction`，`ledger.rs` 与 `risk.rs` 归 `trust`。
2. 模板目录按页面路由命名（维持 1.3 方案）。
3. 不提供 SQLite → PostgreSQL 的升级或迁移工具，跨后端不做数据迁移承诺。
4. 限流统一为进程内令牌桶实现，PostgreSQL 与 SQLite 一致，不使用 Redis 做限流（原文“内容”按“内存”理解，如理解有误请指正）。
5. FTS5 策略：启动时探测，可用则建 FTS5 索引做标题全文搜索，不可用自动降级为标题 LIKE，用户无感知、无需配置。
6. 可执行程序改名（2026-09-04）：`forum-worker` → `bbbcm-worker`、`forum-cli` → `bbbcm-cli`；`AGENTS.md` 命名规则行已同步；`plan/` 事实源中的旧名待设计定稿迁移时一并更新。
7. 前缀改名（2026-09-04）：Redis key 前缀 `sb:` → `bcm:`；数据库 schema 前缀 `bbbcm_` → `bcm_`（保持"以下划线结尾"的约定，实际配置值为 `bcm_`，生成如 `bcm_identity.users`、`bcm_platform.schema_migrations`）。项目代码 `bbbcm` 与二进制名不变；`AGENTS.md` 已同步，`plan/` 事实源旧值待定稿迁移时一并更新。
