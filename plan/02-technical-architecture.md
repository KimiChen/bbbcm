# 02 技术架构、数据与故障边界

## 1. 架构摘要

草莓社区采用 Rust + Axum + Askama SSR + 少量 TypeScript，定位为开源软件，提供两种部署形态：

| 形态 | 数据库 | Redis | 目标 |
|---|---|---|---|
| 完整模式 | PostgreSQL 16 | 可选开启 | 本项目正式部署与中大型社区 |
| 体验模式 | SQLite 单文件 | 不需要 | 开源用户零依赖快速部署体验 |

后续支持 MySQL 8.0（现在不实现，SQL 子集规范先行，见第 6 节，`DEC-022`，目标版本经 `DEC-033` 由 5.7 改为 8.0）。

- 体验模式允许功能降级（降级矩阵见第 6 节），核心社区功能必须完整。
- 后端由配置 `database.backend = "postgres" | "sqlite"` 决定，运行时二选一（`"mysql8"` 为预留值，未实现，见第 4 节）；明确不提供 SQLite → PostgreSQL 的升级或迁移工具，跨后端不做数据迁移承诺。
- 数据库访问统一使用 `sqlx`；搜索为应用层 `SearchBackend` 抽象，完整模式用 `pg_jieba + pg_trgm`，体验模式仅标题检索（FTS5 或 LIKE，见第 13 节）；可靠异步事件使用 Transactional Outbox（各后端同构）。
- Redis 8 承载可丢失、可重建的缓存、短租约、唤醒状态与限流，可选关闭；限流由 Redis 实现，`features.redis = false` 时不提供限流能力（`DEC-024`）。

完整模式拓扑为 Cloudflare 前置、单 VPS 应用层和 PostgreSQL 单主，但具体 VPS、卷和连接池规格在 `DEC-004-B/013` 批准前不设生产默认。

## 2. 代码仓库目标结构

```text
Cargo.toml            workspace 根：统一依赖版本、edition、rust-version、profile
apps/
  web/                可执行：Axum HTTP + Askama SSR（包名 bbbcm-web）
    src/
      main.rs         启动、配置加载、依赖装配
      app_state.rs    数据库连接、缓存/限流实现、Feature Flag、模板上下文
      router.rs       路由挂载点，只组合领域 router，不写业务
      middleware/     会话、CSRF、限流、权限版本、请求日志
      error.rs        AppError 到页面/JSON 响应的映射
      assets.rs       静态资产指纹 manifest 读取
      jobs.rs         worker.embedded=true 时的进程内任务循环
    templates/        Askama 模板，编译进二进制（见 2.2）
    static/           css/ js/ img/ fonts/ 源资产；构建产出带内容哈希的 dist/ 与 manifest.json（见 2.3）
  bbbcm-worker/       可执行：Outbox 消费、邮件、媒体、分析和清理任务
    src/main.rs
    src/jobs/         轮询、租约续期、定时清理
    src/sinks/        邮件、通知、媒体、统计等外部 sink
  bbbcm-cli/          可执行：迁移、种子、账号引导、Feature Flag 管理
    src/main.rs
    src/commands/     db migrate/status/seed、user、flag
crates/
  platform/           配置、SchemaRenderer、Feature Flag、错误、通用类型（Id<T>、游标）
    src/db.rs         Backend 枚举、SQL 占位符渲染、集中方言分支、后端能力集
    src/cache/        CacheStore/Wakeup/Lease/RateLimiter trait（Redis 实现；无 Redis 时缓存走内存、限流不提供）
                      trait 签名在 WP-MVP-A05 接入 Redis 前冻结：届时统一改为返回 Boxed Future 的异步形态并带 Result，
                      现在零调用点，是改签名最便宜的时刻
    src/flags.rs      Feature Flag 唯一注册点
  platform-db/        连接池（双后端，预留 MySQL 8.0 第三成员）、FTS5 探测、迁移发现与多集合对齐校验、事务模板
  platform-outbox/    Outbox 事件模型、lease、sink trait、重试、DLQ（双后端）
  platform-audit/     管理操作审计日志模型与写入
  identity/           领域 crate：用户、凭证、会话、邀请、验证、MFA
  content/            领域 crate：版块、主题、回复、草稿、markdown.rs
  media/              领域 crate：附件、扫描、转码
  interaction/        领域 crate：收藏、关注、反馈、阅读位置、通知（notification.rs）
  moderation/         领域 crate：举报、案件、处罚、申诉
  support/            领域 crate：客服工单、账号恢复
  trust/              领域 crate：信任等级、积分账本（ledger.rs）、风险（risk.rs）
  privacy/            领域 crate：数据请求、删除、导出
  manage/             RBAC/ABAC registry、审批与 /manage 契约
    src/iam/          角色、权限、scope、审批的 registry 与校验
    src/console/      按 /manage 路由区域拆分的页面与数据端点
config/               default.toml 与 development/test/production 脱敏模板
db/
  migrations/
    postgres/         PostgreSQL 迁移集合（0001_init.sql 起）
    sqlite/           SQLite 迁移集合（编号与 postgres 对应同一批结构变更）
    mysql8/           MySQL 8.0 迁移集合（暂为占位，规范先行，对齐校验见 6.3）
scripts/              可重复验证脚本
tests/                集成与 E2E 顶层目录（双后端矩阵）
tests/fixtures/       稳定且脱敏的测试夹具
tests/e2e/            真实浏览器 E2E
```

每个领域 crate 内部固定四层：

- `contract.rs`：请求/响应/视图类型、状态枚举、领域错误码。
- `core.rs`：应用服务与状态机，开启事务、取锁、调用 repo、写 Outbox。
- `repo.rs`：只借用当前事务连接的数据访问，不自行提交。
- `http.rs`：Axum 路由与 handler，只做解析、鉴权调用和响应；文件过大时按资源拆 `http/` 子目录。

### 2.1 API 请求归属规则

| 请求类型 | 路由与 handler 位置 | 说明 |
|---|---|---|
| 公共页面与表单 | `crates/<领域>/src/http.rs` | 由 `apps/web/src/router.rs` 统一挂载 |
| `/manage` 页面与 `/manage/_api/v1/` 数据端点 | `crates/manage/src/console/<区域>/http.rs` | 授权链路按 `03` 执行 |
| 异步外部副作用 | `apps/bbbcm-worker/src/sinks/`（或内嵌 jobs） | 只消费已提交 Outbox |
| 运维命令 | `apps/bbbcm-cli/src/commands/` | 迁移、种子、flag、账号引导 |

Handler 禁止 import `sqlx`；跨领域读取走对方 core 暴露的只读服务，不得跨模块直连他领域表。领域 crate 之间只允许依赖对方的 `contract` 类型与 core 服务，禁止 import 对方 repo。依赖方向：`apps/* → crates/*`；`manage` 可依赖领域 crate 与 `platform-*`；`platform-*` 除可依赖 `platform` 外不横向依赖。

**跨领域写入**：只读服务不足以表达邀请注册这类流程（`identity` 必须在同一事务内写 `trust` 的邀请使用与信任初始状态）。唯一合法形态是：发起方领域的 `core` 在同一 `platform-db` 事务内调用对方 `core` 暴露的**受限写服务**（签名接受当前事务连接，不自行提交），绝不触碰对方 `repo`。

**领域运行时依赖 DAG**（Cargo 禁止循环依赖，本图即编译期约束）：

```text
identity → trust
moderation → content
interaction → content, identity
support → identity
privacy → identity
content → （不依赖其他领域）
media → （不依赖其他领域）
manage → 全部领域
```

反向通知一律走 Outbox（第 11 节），不建立反向 crate 依赖。例如 `trust` 需要感知内容事件时消费 `content` 发出的已提交事件，而不是依赖 `content`。新增任何反向依赖前必须先修订本图；若出现 DAG 与 Outbox 都无法破解的环，作为独立议题重新决策是否把 `contract` 下沉为独立 crate。

### 2.2 HTML 模板

- 模板统一保存在 `apps/web/templates/`，Askama 编译期检查引用，模板编译进二进制（无运行时模板路径）。
- 目录按页面路由划分：`layout/base.html` 骨架 → `partials/`（页头、页脚、分页、表单、错误块）→ `home/ forum/ topic/ auth/ user/ settings/ manage/`。
- 模板只消费 `contract.rs` 暴露的视图类型，不直接接收数据库行或领域内部结构。
- **模板引擎**：Askama 0.16（`DEC-042`）。官方升级说明在上游仓库的 `book/src/upgrading.md`（该仓库没有 `CHANGELOG.md`）。0.16 要求 `rust-version = 1.88`，与本工作区声明一致；默认 feature 为 `config`、`derive`、`std`、`urlencode`，若要用 `|json` 需显式启用 `serde_json`（下划线，0.12 时代叫 `serde-json`）。0.13 移除了 `Template::EXTENSION` 与 `MIME_TYPE` 关联常量，手写 `IntoResponse` 时不能再依赖它们决定 `Content-Type`。

  从 0.12 累计的**模板语法破坏性变更**，重做 `WP-MVP-A00` 时逐条对照：

  | 变更 | 说明 |
  |---|---|
  | `{% call %}` | 自 0.14 起是块标签，必须配 `{% endcall %}`；0.12 的自闭合写法直接报错 |
  | `{% let x %}`（无 `=`） | 改为块语法并配 `{% endlet %}`；只声明不赋值改用 `{% decl x %}` |
  | `{% set %}` | 同上，需配 `{% endset %}` |
  | 位运算符 | 竖线、`&`、`^` 改名为 `bitor`、`bitand`、`xor`（竖线被过滤器占用） |
  | 变量名 | 不得以 `__askama` 开头、不得是 Rust 关键字、不得叫 `caller` |
  | 重名 `{% block %}` | 同一模板内重复的 block 名现在报错 |
  | `json` 过滤器 | 默认不再美化输出，要缩进写 `json(2)` |
  | `linebreaks` 系列 | 现在自动转义输入 |
  | `as_ref` 过滤器 | 改名为 `ref` |
  | 已删除的过滤器 | `abs`、`into_f64`、`into_isize`、`markdown`、`yaml` |

  空白控制默认值未变（`preserve`，只有显式写 `-` 才裁剪），`{%- -%}` 语义跨版本一致。自定义过滤器自 0.15 起必须带 `#[askama::filter_fn]` 属性宏并接收第二个 `&dyn askama::Values` 参数——0.12 那种「在 `filters` 模块里写自由函数」的写法会编译失败。
- **Web 集成**：**不引入** `askama_axum`（已废弃，最新版是无依赖的墓碑包，最后可用版本只支持 askama 0.12 + axum 0.7）与 `askama_web`（可用，但其 `IntoResponse` 在渲染失败时返回硬编码英文 `INTERNAL SERVER ERROR` 的 `text/plain`，会绕过 `apps/web/src/error.rs` 已建立的 `AppError` 状态码映射与中文文案）。集成方式固定为：handler 返回 `Result<Html<String>, ApiError>`，由 `router.rs` 的 `render()` 辅助函数把 `Template::render()` 的错误转成 `AppError::Internal`。该签名在 0.12 与 0.16 下一致。
- **模板根**：Askama 按派生所在 crate 的 `CARGO_MANIFEST_DIR/templates` 解析路径，领域 crate 无法直接引用 `apps/web/templates/`。凡渲染页面的领域 crate 在 crate 根放 `askama.toml`（`[general]` 下 `dirs = ["../../apps/web/templates"]`，该键自 0.16 起支持 glob）并依赖 `askama`；**不得在领域 crate 内新建 `templates/` 目录**，否则模板结构会分裂成多份。
- **错误态与空状态**由页面视图类型的必填字段在编译期强制，而非模板语法：所有页面视图内嵌 `PageFrame { flash, error, empty }`，`layout/base.html` 固定 `{% include "partials/error_block.html" %}`，该 partial 引用 `frame.error` 与 `frame.empty`；Askama 在编译期校验模板引用的字段存在，缺字段即编译失败。列表类视图统一 `ListView<T> { items, empty }`，空状态文案是必填字段。模板不得自行省略这两个块。

### 2.3 静态资产

- 源资产在 `apps/web/static/{css,js,img,fonts}/`，构建时生成内容哈希文件名与 `manifest.json`。工具链为 **Node + esbuild**（`DEC-032`）：Node 与 npm 版本、`package-lock.json`、前端依赖的 SBOM 与许可证扫描纳入 `WP-M0-03` 的供应链锁定；构建脚本归 `WP-MVP-A01` 交付，产物落 `apps/web/static/dist/`，源路径不入生产。运行时可替换的品牌资产由覆盖层补充进 manifest（`DEC-035`，见 `05` 第 4 节）。
- 构建前置配置与复现入口为根目录 `.node-version`、`package.json`、`package-lock.json`、`.npmrc` 和 `supply-chain/lock.json`，操作说明见 `supply-chain/README.md`。M0-03 与 A01 共用，A00 的全局采集 Node 版本不自动成为构建基线；本轮不新增资产构建产物或修改 A00 冻结结构。
- `apps/web/src/assets.rs` 提供模板 helper 按 manifest 解析指纹路径；指纹路径响应 `Cache-Control: immutable, max-age=1y`，源路径不入生产。TypeScript 很少量，仅增强交互，核心流程无 JavaScript 可用。

### 2.4 其他基础设定

- 强类型 ID：`platform::types::Id<T>` 按 ID 属主类型区分，JSON 以十进制字符串传输；列表分页统一签名 keyset 游标。
- 错误模型：`platform::AppError` 唯一错误出口，`apps/web/src/error.rs` 统一映射 HTTP 状态与错误页；领域错误码定义在各自 `contract.rs`。
- Feature Flag：`crates/platform/src/flags.rs` 唯一注册点，默认全关，新增 flag 必须先登记再引用。
- 格式化与静态检查：`rustfmt.toml` 固定格式，Clippy `-D warnings` 本地强制。验证时机按 `DEC-020` 分层：开发期每次提交只执行最小集（格式化、Clippy `-D warnings`、单元测试、SQLite 冒烟、`git diff --check` 与暂存区复核），真实 PostgreSQL/Redis 集成、双后端矩阵、安全、并发、性能与视觉证据统一到发布前硬化阶段执行。
- 禁止创建第二个数据库访问层或绕过领域服务的 HTTP SQL 路径。

## 3. 运行组件

### bbbcm-web

- 公开 SSR、认证页面、用户设置和 `/manage`。
- 只解析和校验传输输入，不直接写 SQL。
- 写请求创建事务并调用领域服务；响应仅在提交结果明确后返回成功。
- JavaScript 只增强体验，关键浏览、分页、登录和表单在无 JavaScript 时可用。
- SQLite 体验模式 `worker.embedded = true` 时同时承担任务循环宿主；PostgreSQL 必须使用独立 Worker 及其专用角色，启动校验拒绝 PostgreSQL 内嵌模式，避免 Web 凭证承担 Worker 数据权限。

### bbbcm-worker

- 轮询 Outbox，消费已提交事件并分发到 sink。
- 承担媒体转码、定时清理和聚合任务。
- 程序名固定为 `bbbcm-worker`；不写业务规则，引擎在 `platform-outbox`。

### bbbcm-cli

- `db migrate --backend <postgres|sqlite>`、`db status`、`db seed`。
- `user create/promote/reset-password` 账号引导与救援。`user promote` 只在系统尚无任何特权账号时可用于创建首个站点所有者（`AUTH-014`），此后必须拒绝执行；每次调用写入追加式审计并触发告警。
- `flag list/set` 受控 Feature Flag 修改。
- 程序名固定为 `bbbcm-cli`；同样是薄壳，调用 crate 服务，保证 web/cli/worker 共用同一套逻辑。

## 4. 配置与 Secrets

配置优先级为：安全默认值、版本化模板、环境变量、部署密钥注入。生产环境不得使用开发演示开关。

关键配置键：`server.environment`（`development|test|staging|production`）、`server.bind`、`database.backend`、`database.schema_prefix`（默认且生产固定 `bcm_`，与 `01` §7.2 一致）、`database.sqlite_path`、`database.max_connections`、`features.redis`、`worker.embedded`。全部配置结构使用 `deny_unknown_fields`：拼错的键必须拒绝启动，而不是静默取默认值。`database.backend` 取值为 `"postgres" | "sqlite"`，预留 `"mysql8"`（未实现：配置该值时拒绝启动并提示该后端尚未接入）。PostgreSQL 连接串只经环境变量 `BBBCM_DATABASE_POSTGRES_URL` 注入，不入任何配置文件；本地覆盖文件 `config/local.toml` 不入库。

必须验证：

- 环境只能是 `development/test/staging/production`。
- 数据库 URL、Redis URL、schema 前缀和所有外部 provider 配置完整性。
- 未批准的供应商配置不存在时保持关闭，不填写虚构默认值。
- staging/production 拒绝 demo 模式、弱 cookie、默认密钥和未解析占位符。
- 完整模式启动时查询 `pg_extension` 确认 `pg_jieba` 与 `pg_trgm` 已安装；缺失时拒绝启动，或按已批准的降级语义关闭 `search` flag，不得静默降级到未评审的检索路径。
- 数据库须以 UTF-8 且非 `C`/`POSIX` 的 `lc_ctype` 创建：`pg_trgm` 的中文行为依赖 ctype，启动时校验 `pg_database.datctype`，不满足时拒绝启动。
- Secret 不进入日志、错误响应、操作日志、Outbox 或版本库。

## 5. Feature Flag

所有 flag 默认关闭，只有本地测试夹具可以显式打开 MVP 能力。MVP 允许定义：

- `public_read`
- `registration`
- `email_verification`
- `topic_creation`
- `reply_creation`
- `attachments`
- `search`
- `rss`
- `reporting`
- `support`
- `manage_console`
- `registration_lock`
- `invitation_lock`
- `read_only_mode`

后续阶段 flag 包括积分、签到、私信、投票、悬赏、商家交易、抽奖、红包、开放 API、Webhook、外部桥接、AI、生产 JIT 等，M0/MVP 必须保持关闭且不可达。

数据库中的 flag 只允许两个受控入口修改（`DEC-028`）：`/manage/config`（经 `ADMIN-011` 审批链，高危 flag 双人审批）与 `bbbcm-cli flag set`（面向无 HTTP 可用的引导与救援场景）。两者共用 `platform` 层同一写命令与同一审计路径；紧急关闭优先于普通开启。

## 6. 数据库后端与多库兼容

### 6.1 抽象层约定

- `platform::db::Backend` 枚举 `Postgres | Sqlite`（预留 `Mysql8`，未实现）；`platform-db` 提供双后端连接池与事务模板；后续接入 MySQL 8.0 时 `sqlx` 增加 `mysql` feature，Pool 枚举扩展第三成员。
- repo 中 SQL 一律写 `?` 占位符，`platform::db::render_sql(backend, sql)` 在 PostgreSQL 下转换为 `$1..$n`（引号内不转换），SQLite 下原样；MySQL 占位符同为 `?`，未来仍只需 PostgreSQL 一个转换分支；不使用 `sqlx::query!` 编译期宏。
- 可移植 SQL 子集为 PostgreSQL 16 + SQLite + MySQL 8.0 三库交集（`DEC-022`，目标版本经 `DEC-033` 修订）：`SELECT/INSERT/UPDATE/DELETE`、JOIN、子查询、`LIMIT`，以及 **CTE（`WITH`）与窗口函数**——二者在 PostgreSQL 16、SQLite 3.25+ 与 MySQL 8.0 均可用，是把目标版本从 5.7 抬到 8.0 换来的主要收益。
- 子集之外明确禁用：`RETURNING`（MySQL 8.0 仍无）、`ON CONFLICT ... DO UPDATE`（MySQL 对应方言为 `ON DUPLICATE KEY UPDATE`，不在交集内）、部分索引（MySQL 无）。upsert 统一为 update→insert 两步，或集中在 `platform::db` 的方言分支函数。方言分支的标准写法为 `SAVEPOINT` + INSERT 优先：`SAVEPOINT sp` → `INSERT` → 捕获唯一冲突（PostgreSQL `23505`／SQLite `2067`、`1555`／MySQL `1062`）→ `ROLLBACK TO SAVEPOINT sp` → `UPDATE`。必须用保存点的原因是 PostgreSQL 在事务内遇唯一冲突会把整个事务置为 aborted（后续语句返回 `25P02`），而 SQLite 与 MySQL 只回滚该语句——三库行为不一致，不设保存点则同一段代码在 PostgreSQL 下会丢掉整个事务。唯一冲突属业务冲突，不与 `40001`／`40P01` 共用重试预算。
- 主键回读走平台层 `last_insert_id` 抽象，repo 不手写方言回读。三库实现集中在 `platform::db` 方言分支：PostgreSQL 在同一事务同一连接上 `SELECT lastval()`（或 `currval(pg_get_serial_sequence(...))`），每次只回读一条刚插入的行；SQLite `last_insert_rowid()`；MySQL 8.0 `LAST_INSERT_ID()`。批量插入不使用该抽象，改为应用层预先确定唯一键后回查。
- 方言差异只允许集中在 `platform::db` 方言分支模块（`match backend`）：PostgreSQL 专属语法（`ILIKE`、`FOR UPDATE`、`SKIP LOCKED`、`::cast`）只能出现在方言分支内，repo 层不见方言。
- 锁策略：PostgreSQL 用 `FOR UPDATE` 行锁，批量租约可用 `SKIP LOCKED`（MySQL 8.0.1+ 同样支持，SQLite 单写者不需要、为空实现分支）；SQLite 写事务一律由 `platform-db` 事务模板以 `BEGIN IMMEDIATE` 开启（`DEFERRED` 事务在 WAL 下读后升级写会立即返回 `SQLITE_BUSY_SNAPSHOT`，`busy_timeout` 对该错误无效），依赖单写者 + WAL + `busy_timeout`；MySQL 8.0 行锁与 `SKIP LOCKED` 语义与 PostgreSQL 同构，接入时确认。
- 标识符经 `SchemaRenderer` 渲染：PostgreSQL `bcm_<schema>.<table>`，SQLite/MySQL `bcm_<schema>_<table>`（`DEC-029`）。两种形式机械双向推导，避免 15 个逻辑 schema 压进同一命名空间时 `sessions`、`jobs`、`evidence` 等表名跨域相撞，也保证与同库其他应用隔离。迁移元数据表相应为 `bcm_platform.schema_migrations`（PostgreSQL）与 `bcm_platform_schema_migrations`（SQLite/MySQL）。`SchemaRenderer` 内置受信 `(schema, table)` 注册表，渲染前校验参数在册。
- 类型读写转换（时间、布尔、JSON 文本）统一封装在 `platform::db`，repo 不手写。硬约束：时间的编解码只允许经 `platform::db` 的转换入口，禁止 repo 直接把 `DateTime<Utc>` 绑定到 SQLite 列或手写 `to_rfc3339`；SQLite 轨的时间列一律 `TEXT NOT NULL`。
- 不引入 sea-orm、diesel、rbatis 等 ORM。

#### 语义差异禁用清单（三库语法合法但结果不同）

上一节禁的是「某库不支持」，本节禁的是「三库都能跑但结果不一样」——后者不会报错，只会在换后端时静默出错，危害更大。

| 禁止 | 原因 | 替代写法 |
|---|---|---|
| 双引号标识符 | PostgreSQL 区分大小写，MySQL 默认把双引号当字符串（除非 `ANSI_QUOTES`） | 一律经 `SchemaRenderer` 渲染 |
| `\|\|` 字符串拼接 | MySQL 默认把 `\|\|` 解释为逻辑或 | 拼接在应用层完成 |
| `UPDATE ... FROM` / `UPDATE ... JOIN`、多表 `DELETE` | 三库语法各异 | 改子查询，或应用层两步写 |
| `NULLS FIRST/LAST` | MySQL 不支持；三库 NULL 默认次序不同 | 参与 `ORDER BY` 或 keyset 游标的列一律 `NOT NULL` |
| `CAST` | 溢出与舍入行为不同 | 类型转换在应用层 |
| `IS DISTINCT FROM` | MySQL 无 | 显式 `IS NULL` 组合 |
| `IN` 子查询内含 `LIMIT` | MySQL 8.0 仍报 1235 | 先普通 `SELECT` 取 id 批，再 `UPDATE ... WHERE id IN (?,?,...)` |
| 裸 `LIKE` | 大小写敏感性随 collation 变 | 只作用于应用层规范化列，且必须带 `ESCAPE` |

该清单由 `WP-MVP-A03` 交付的共享 SQL lint（`platform` 下的 `#[test]`，扫描 repo 层 SQL 常量）强制，属 `DEC-020` 最小集内的单元测试。
- 全局禁用清单（三库通用）：触发器、存储过程、视图、部分索引（MySQL 无）、数据库端 UUID/随机 ID 函数（如 `gen_random_uuid()`）。自增主键由数据库序列/自增列生成并经 `last_insert_id` 抽象回读；事件与操作 ID（UUIDv7）在应用层生成。

### 6.2 类型映射

| 概念 | PostgreSQL | SQLite | MySQL 8.0（预留） |
|---|---|---|---|
| 主键 | `BIGINT IDENTITY` | `INTEGER PRIMARY KEY AUTOINCREMENT`（禁止 rowid 复用） | `BIGINT AUTO_INCREMENT`（8.0 起计数器跨重启持久化，5.7 的回退隐患不再存在） |
| 时间 | `timestamptz` | `TEXT`（RFC3339，UTC，微秒精度固定宽度） | `DATETIME(6)` + UTC 约定 |
| 布尔 | `boolean` | `INTEGER` 0/1 | `TINYINT(1)` |
| 枚举/状态 | 文本 + `CHECK` + 应用层白名单 | 文本 + `CHECK` + 应用层白名单 | 文本 + `CHECK` + 应用层白名单（MySQL 8.0.16+ 起真实生效） |
| JSON 载荷 | `TEXT`（应用层解析校验，不使用 `JSONB` 运算符） | `TEXT`（应用层解析校验） | `TEXT`（应用层解析校验） |
| 事件/操作 ID | UUIDv7 文本 | UUIDv7 文本 | UUIDv7 文本 |
| 全文索引 | `pg_jieba` GIN + `pg_trgm` | FTS5（可选）或标题 LIKE | `FULLTEXT + ngram`（接入时实现） |
| 迁移元数据 | `bcm_platform.schema_migrations` | `bcm_platform_schema_migrations`（扁平） | `bcm_platform_schema_migrations`（扁平） |

#### 时间文本编码规范

时间在 SQLite 以文本存储，格式一旦漂移就会破坏排序与 keyset 游标，且开发期只跑 SQLite 冒烟、不会暴露跨库差异。本段是唯一权威定义：

- 应用层统一使用 `to_rfc3339_opts(SecondsFormat::Micros, true)`，即固定 27 字节、6 位小数、大写 `Z` 后缀、UTC，例如 `2026-09-05T04:00:00.000000Z`。固定宽度保证字典序等于时间序。
- 三库统一微秒精度：PostgreSQL `timestamptz` 原生微秒，SQLite `TEXT`，MySQL 8.0 `DATETIME(6)`。
- 编解码入口是 `platform::db` 的 `encode_time`/`decode_time`，附固定格式单元测试；`platform-outbox` 与 `platform-audit` 的时间字段注释引用本段，不另行定义。

字符串唯一键与比较统一二进制语义：MySQL 8.0 列使用 `utf8mb4_0900_bin` collation，SQLite 使用 `BINARY`，PostgreSQL 默认即二进制语义；大小写与空白规范化由应用层负责。MySQL 默认 collation（8.0 为 `utf8mb4_0900_ai_ci`）大小写不敏感，是三库行为差异中最隐蔽的坑，登录名、邀请码、唯一键等比较场景必须点名防范。

### 6.3 迁移 N 轨

- `db/migrations/{postgres,sqlite,mysql8}/` 三套有序文件，同一编号代表同一批结构变更；`mysql8/` 现在只放占位，结构规范先行（`DEC-022`／`DEC-033`）。
- `bbbcm-cli db migrate --backend <postgres|sqlite>` 选择集合执行；`--backend mysql8` 为预留值，未实现时拒绝执行并提示。
- `platform_db::migrations::verify_alignment` 做多集合编号对齐校验：已填充的迁移集合编号必须一一对齐；`mysql8/` 占位目录在真实迁移落地前不参与对齐。
- 纯 ACL 迁移的配对形式（`DEC-065`）：某一轨确实无对应语义时（典型是 PostgreSQL 专有的 `GRANT`／`REVOKE`，SQLite 没有 schema 与角色级权限），该轨文件仍须存在且编号对齐，但首行写声明标记 `-- BBBCM-TRACK-NOOP:` 并说明原因，此时豁免「至少含一条可执行 SQL」的要求。该标记只用于本轨确实无事可做的情形，不得用来跳过本应实现的结构变更。
- SQLite 连接统一 `journal_mode=WAL`、`busy_timeout=5000ms`、`foreign_keys=ON`（连接池建立时自动启用）。
- 迁移执行器规范：迁移一律在专用单连接上执行，不经业务连接池；经 `SchemaRenderer` 渲染的迁移元数据表（PostgreSQL `bcm_platform.schema_migrations`，SQLite/MySQL `bcm_platform_schema_migrations`）以编号幂等判定作为并发防护第一道，重复应用必须被拒绝。
  - PostgreSQL：每个迁移在其事务内调用 `pg_advisory_xact_lock(hashtext(<schema 前缀>))`。用事务级（`xact`）而非会话级，锁随事务自动释放；锁键含 schema 前缀，避免同库双前缀测试互相阻塞。
  - SQLite：每个迁移以 `BEGIN IMMEDIATE` 开启（「单写者免锁」不成立——同一文件上并发 `db migrate` 属误操作，须被幂等判定拒绝）。重建型迁移在文件头标 `-- rebuild`，执行器对该类迁移在事务外先 `PRAGMA foreign_keys=OFF`，事务内结束前跑 `foreign_key_check`，退出时恢复 `ON`。
  - MySQL 8.0（接入时定）：原子 DDL 保证单条 DDL 不会半应用，但 DDL 仍隐式提交、无法在事务内回滚；每条 DDL 前查 `information_schema` 做幂等判定，并用 `GET_LOCK` 串行化。

### 6.4 Outbox 与 worker 差异

- Outbox 表结构各后端同构，事件随主库事务提交，语义一致。
- PostgreSQL worker：`FOR UPDATE SKIP LOCKED` 批量租约，多 worker 安全。
- SQLite worker：单写者租约，采用“取 id 批 → UPDATE 租约 → 按批次回读”三步写法——先普通 `SELECT id ... WHERE 可租约 LIMIT ?` 取一批，再 `UPDATE ... SET lease_owner = ?, lease_batch_id = ?, lease_until = ? WHERE id IN (?,?,...) AND 可租约` 并核对影响行数，最后 `SELECT ... WHERE lease_batch_id = ?` 回读。不依赖 `RETURNING`。
  - `lease_batch_id` 为应用层生成的 UUIDv7 文本列：按 `lease_owner` 回读会串批（同一 worker 的上一批未清理时会被一起读出），必须按批次回读。
  - `lease_owner` 取 `hostname:pid:随机`，用于诊断与租约回收，不作为回读条件。
  - 不使用 `UPDATE ... WHERE id IN (SELECT ... LIMIT ?)`：MySQL 8.0 仍对该写法报 1093/1235，不满足三库交集。
  - SQLite 后端下 `worker.embedded = true` 与独立 `bbbcm-worker` 进程互斥，由启动期校验强制，不再只是「建议」。
- MySQL 8.0：支持 `FOR UPDATE ... SKIP LOCKED`，租约与 PostgreSQL 同构（`DEC-033`），接入时复用同一方言分支即可，不再需要单 worker 或 `GET_LOCK` 变通。

### 6.5 功能降级矩阵

| 能力 | PostgreSQL 完整模式 | SQLite 体验模式 |
|---|---|---|
| 搜索范围 | 标题 + 正文，中文分词、模糊匹配 | 仅标题（FTS5 可用时用 FTS5，否则 LIKE） |
| 相关性排序 | jieba 权重 + trgm | 按时间/精确匹配简单排序 |
| 并发写入 | 多 worker、多实例 | 单写者，建议单 worker |
| 容量定位 | 1 万用户 / 10 万帖基线（`DEC-070`） | 小规模体验（数百用户 / 数万帖量级） |
| 限流 | Redis 令牌桶（`features.redis = true`） | 无（体验模式默认无 Redis，不提供限流） |
| 缓存 | Redis 共享缓存（开启时） | 进程内 LRU 或直接回库 |
| 领域功能 | 完整 | 完整（同一套 core/repo 代码） |

完整模式的搜索能力取决于扩展是否就位：`pg_jieba` 或 `pg_trgm` 缺失时，`search` flag 关闭并在搜索入口返回受限提示，不回退到未评审的检索路径；`lc_ctype` 为 `C`/`POSIX` 时拒绝启动。

MySQL 8.0 接入后与完整模式同级定位，搜索使用 `FULLTEXT + ngram`；当前不实现，规范先行（`DEC-022`／`DEC-033`）。

FTS5 探测：启动时在 temp 库尝试创建探针虚表，成功即判定可用并随即删除，失败只记一行日志、不阻断启动；结果缓存进后端能力集，运行期不重复探测。

### 6.6 版块 slug 保留字

`DEC-068` 把版块页放到顶级 `/{slug}`，版块 slug 因此与站点级静态路由共用同一段命名空间。路由匹配时静态段优先于参数段，所以取用已注册路径段的版块不会报错，而是**永远匹配不到自己的页面**——一个没有任何错误信号的失效。必须在写入时拒绝，不能留到运行期观察。

保留字为「全部已注册的顶级静态路径段」，由路由表推导而非另抄一份常量，避免新增路由时清单漏更。当前取值：

```
health  status  assets  search  new  pinned  featured  hot  tags  t  topics
posts   me      u       login   register     logout    settings      manage
forums  media   feed.xml       robots.txt    sitemap.xml   content
polls   bounties      lotteries      red-packets    top    verified
rules   terms   privacy  __fixtures  __diagnostics
```

清单同时包含尚未实现但已在 `01` 第 10 节登记的路由（`polls`、`top`、`verified` 等）：先占位比日后为了让路由上线而改动线上版块 slug 代价低得多。

校验点有两处，缺一不可：种子写入（`seed_repo`）和将来的建版块入口。两处都拒绝保留字、拒绝空串与非 `[a-z0-9-]` 字符。
### 6.6-A 搜索索引轨（`postgres-search`）

`02` 第 258 行的存储映射写明全文索引为「`pg_jieba` GIN + `pg_trgm`」，但索引不能放进常规迁移轨：GIN 表达式索引必须写 `to_tsvector('public.jiebaqry', …)`，缺 `pg_jieba` 的实例执行该 DDL 直接失败，而 §6.5 明确允许这类实例正常启动、只关闭搜索能力。放进常规轨等于把扩展升格为**迁移期硬依赖**，那条降级路径就没了。

因此索引单独成一条轨：

| 项 | 约定 |
|---|---|
| 目录 | `db/migrations/postgres-search/` |
| 编号对齐 | **不参与**三库基线的对齐校验（不在 `TRACKS` 内），但共用同一套 `validate_content` |
| 执行方式 | 显式命令 `bbbcm-cli db search-index`，不随 `db migrate` 自动执行 |
| 缺扩展时 | **跳过并打印缺失扩展名，退出码 0**——这是 §6.5 允许的降级状态，不是失败 |
| 历史表 | `platform.search_migrations`，与 `schema_migrations` 分开：两者适用的实例集合不同，混表会让「基线已就绪」与「搜索已就绪」无法区分 |
| 幂等 | 已记录且校验和相同则跳过；不同则报 `ContentDrift` 拒绝执行，必须用新的前向迁移 |

索引表达式必须与 `platform::db::full_text_search_sql` 生成的条件逐字一致，否则规划器不会选中它。当前为 `title` 与 `body` 两个独立表达式索引——查询里两者是 OR 关系且分列两个字段，合并成一个索引无法服务只命中其中一列的查询。

配置名写全限定 `public.jiebaqry`：表达式索引要求函数不可变，两参数形式的 `to_tsvector(regconfig, text)` 才是 IMMUTABLE，依赖 `search_path` 的单参数形式是 STABLE，建索引会直接失败。

SQLite 体验模式没有这条轨，`platform.search_migrations` 不应出现在 SQLite 库里（`seed.rs` 有反向断言）。

## 7. PostgreSQL 逻辑 schema

默认物理前缀为 `bcm_`，同时支持测试中的受信非默认前缀；SQLite/MySQL 使用 `<前缀><逻辑 schema>_<表名>`（`DEC-029`），逻辑 schema 到物理表名的映射由 `SchemaRenderer` 负责，并对 `(schema, table)` 参数做受信注册表校验。15 个逻辑 schema：

| 逻辑 schema | 责任 |
|---|---|
| `platform` | Feature Flag、幂等、Outbox、作业、运行状态 |
| `identity` | 用户、凭证、会话、邀请、同意、MFA |
| `iam` | 管理角色、权限、分配、范围、审批和管理会话 |
| `community` | 版块、标签、主题、回复、修订、草稿和搜索文档 |
| `media` | 附件、变体、引用、隔离和回收 |
| `interaction` | 收藏、关注、反馈、阅读位置和通知 |
| `trust` | 信任、资格、风险和邀请归因 |
| `campaign` | 后续活动底座，MVP 关闭 |
| `commerce` | 后续商业底座，MVP 关闭 |
| `moderation` | 举报、案件、证据、处罚和申诉 |
| `support` | 客服、账号恢复和紧急升级 |
| `privacy` | 数据请求、导出、删除墓碑和传播 |
| `operations` | 管理工作项和事故记录 |
| `audit` | 追加式管理操作审计日志 |
| `analytics` | 脱敏事件、聚合、字典和排除规则 |

代码不得硬编码物理 schema 或拼接不受信标识符。Schema/Table 只能经受信 `SchemaRenderer` 渲染，所有值使用 bind 参数。

## 8. 数据库角色与 ACL（PostgreSQL）

本节仅适用 PostgreSQL 完整模式。角色与 ACL policy 在 `WP-MVP-A03` 随迁移一并**实施**，此后每张新增表与序列必须在同一迁移提交中加入 ACL 语句；真实 PostgreSQL 的未来对象探针与权限继承**验证**按 `DEC-020` 统一在发布前硬化阶段执行。SQLite/MySQL 不适用，MySQL 账号权限模型接入时另定。

最小角色集合：

- `migrator`：拥有应用 schema 对象，执行迁移，不用于服务运行。
- `app_rw`：Web 最小读写权限，不得修改 IAM catalogue 或审计历史。
- `worker_rw`：仅访问明确列入 worker policy 的表。
- `report_ro`：只读取脱敏分析列。

规则：

- runtime 角色 `NOINHERIT`，无 schema `CREATE`。
- 默认 ACL 不自动向 runtime 或 `PUBLIC` 授权未来对象。
- 每张新增表和序列必须显式加入 ACL policy。
- 审计历史不可被 app/worker/report 角色更新或删除。
- IAM catalogue 对应用运行角色只读；授权状态变更通过受控命令和专用权限执行。
- 真实 PostgreSQL 测试必须创建未来对象探针，验证无意授权不会继承。

SiteOwner 与唯一权限 registry 已由 `DEC-058` 批准（关闭 `DEC-016`／`DEC-018`／`CR-011`／`CR-013`）：

- 仓库内单一 TOML registry 文件是角色、权限与配置键的唯一事实源，位于 `crates/manage/src/iam/`；构建期由它生成 Rust 枚举与数据库种子，三者任一不一致时启动拒绝。不得手写与 registry 并行的权限常量，不得建立多套同义角色。
- SiteOwner 权限集固定为密钥轮换、账单与供应商、生产访问授权、灾备恢复四类；禁止 `all permissions` 通配，不含内容、用户、审核、客服日常权限，也不含紧急内容下架。
- 生产种子仍需 `WP-MVP-D01` 的实现与审批链接线，本条只解除决策阻塞。

## 9. Repository 与事务规则

- Handler 不写 SQL。
- Repository 只借用当前事务连接，不自行提交。
- 业务状态、版本、幂等结果和 Outbox 在同一事务提交。
- 锁顺序固定并文档化，跨领域事务不反向取锁。
- 可重试错误集按后端分类，只在命令幂等且重试预算内重试：PostgreSQL `40001`（序列化失败）、`40P01`（死锁）；SQLite `SQLITE_BUSY`(5)、`SQLITE_BUSY_SNAPSHOT`(517)；MySQL 8.0 `1213`、`1205`（接入时定）。唯一冲突（`23505`/`2067`/`1062`）属业务冲突，不进入重试预算，按 §6.1 的保存点写法处理或返回 `409`。重试与幂等键的关系不变：无幂等键的命令不得重试。
- 客户端可见成功必须对应确定提交；提交结果未知时返回可重试状态并通过幂等键查询结果。
- 所有更新使用版本或 `If-Match` 防止静默覆盖。
- 删除、隐藏、权限收紧与权威搜索状态同事务更新。

## 10. 关键原子流程

### 邀请注册

锁邀请码和创建者配额，校验状态、过期和唯一消费；创建用户、凭证、资料、同意记录、信任初始状态、邀请使用、幂等结果和 Outbox 后一次提交。

### 邮箱验证与密码恢复

令牌仅存哈希、单次消费、短时有效。密码更新撤销其他会话并写安全事件。未知、锁定、删除或错误账号返回统一语义并执行固定成本验证路径。

### 内容创建

锁定用户和版块状态，校验邮箱验证、限制、可见性、速率、幂等和附件终态；创建主题/回复、稳定楼层、修订、权威搜索文档和 Outbox 后提交。

DEC-052内容切片的受信装配：`content::contract::ContentAuthority` 由 apps 实现，接受发起方当前事务连接，调用 identity/moderation/trust 的受限 core 服务；content 不反向依赖这些领域、不访问他域 repo，platform 子包不增加横向依赖。新写锁序为幂等、identity.users、identity.sessions、trust.identity_snapshots、community.commit_state、topics、posts、moderation 案件顾问锁（`DEC-066`，按 `(target_kind, target_id)`）；所有可变锁结束后追加风险、分析和 Outbox。版块不参与加锁（`DEC-064`）：它是迁移/种子维护的参照数据，运行角色只有 `SELECT`，且 `community.commit_state` 根锁已把所有社区写串行化；版块链的可见性、启用状态与父链环检测仍在根锁之后校验，只是不取行锁。限制变更的未来受控命令必须使用同一 identity.users 根锁。风险四维预检查在这些数据库锁之前执行，其 opaque context 在写事务内再次绑定当前账号、动作和规则版本；它不能代替邮箱、会话或信任重读。

内容、修订、永久楼层、严格发布消费事实、权威搜索投影、幂等结果共同提交。TopicPublished 的资源 ID 为主题 ID，ReplyPublished 为回复 post ID；只表示原始成功发布，不表示首次行为或有效贡献。作者隐私、发布参数及计数按DEC-054：作者列表与正文对未登录访客默认公开且用户可关闭；标题5–120、正文10–50000、回复2–10000、标签≤5，均按Unicode标量；本人编辑/删除窗口默认30分钟；严格配额在本事务内消费，UTC零点分日，成功与待审均计入、本人删除不返还。内容动作四维反滥用预算与外链门槛按DEC-060，令牌桶语义沿用DEC-051且与严格配额并存。普通编辑只能保留原发布/审核/讨论/归档状态并按权限调整主题选择；本人软删除仅在显式批准的删除政策下设置墓碑；删除态主题按DEC-061继续可读并保留在列表与搜索投影中，仅首帖正文以占位呈现，moderation轴的隐藏语义不变，审核、锁定、恢复和移动命令不在本切片开放。

分页顺序使用已提交单调水位与不可变主题排序历史，community 根锁避免不同事务获取序列后乱序提交造成漏项。回复按永久 floor 升序、每页50条，水位之后的插入留到新遍历；每页重读当前权限，权限收紧产生的安全移除不视为分页缺陷。排序历史、修订、消费事实不自行增加清理期限。游标签名绑定视图、筛选、主体和水位；主题楼层永久定位不是移帖跳转。缓存仅保存权限重读后的公开正文，版本key/30秒TTL和失效Outbox沿A05；站内链接、引用、成员与待审正文绕过公开缓存。

### 举报与治理

举报、证据快照、案件、处罚、申诉、操作状态和 Outbox 按命令边界原子提交。申诉复核人与原处罚操作者分离。

### 隐私删除

确认、冷静期、撤销、执行墓碑、内容匿名化、搜索收紧、附件清理任务和 Outbox 保持可恢复状态机。法律保留覆盖删除时必须记录范围、依据和到期。

## 11. Transactional Outbox

Outbox 事件包含事件 ID、聚合类型/ID、事件类型、版本、发生时间、最小脱敏 payload 和追踪 ID。不得包含密码、令牌、邀请码明文、邮件正文、证件或原始 C3 数据。

每个 sink 使用 `(sink, event_id)` 唯一键保证幂等。Worker 支持 lease、续租、重试、退避、DLQ、人工重放和停用外部副作用。业务数据不能依赖 Worker 才变得正确。

## 12. Redis 可选

Redis key 固定为 `bcm:<environment>:v1:<domain>:<key>`。允许存放：

- 短租约和 worker 唤醒。
- 可重建公开缓存。
- 非权威在线状态。

禁止存放：用户、邀请、权限、内容、审批、审计、删除墓碑或幂等结果的唯一副本。清理脚本只清理当前环境 namespace，使用 `SCAN` + `UNLINK`，不得 `FLUSHDB`。

装配规则：

- `features.redis = true`：装配 Redis 实现，缓存共享、worker 唤醒低延迟。
- `features.redis = false`：装配进程内存实现——缓存改进程内 LRU、唤醒改固定间隔轮询。
- 限流由 Redis 承载（`DEC-024`）：键在 `bcm:<env>:v1:ratelimit:<维度>:<标识>` 下，多实例共享同一计数，是唯一“丢失即降低安全姿态”的 Redis 用途。`features.redis = false` 时不提供限流能力——SQLite 体验模式默认即此配置，因此无注册、登录、找回速率保护，仅适用于本机体验与评估，不得公开部署。`crates/platform/src/cache/ratelimit.rs` 的进程内令牌桶仅供测试与本机开发，不是生产限流器。
- 会话、幂等记录、业务事实始终在主库，Redis 关闭不影响正确性。

## 13. 搜索

- 搜索统一经应用层 `SearchBackend` 抽象：完整模式由 PostgreSQL 16 实现，安装并锁定 `pg_jieba`、`pg_trgm` 版本；SQLite 体验模式用 FTS5（启动探测）或 LIKE；MySQL 8.0 接入时用 `FULLTEXT + ngram`（后续实现，规范见第 6 节）。
- `search_documents` 是权威投影，与主题/回复状态同事务更新。
- 查询先执行可见性、审核、删除、版块和拉黑过滤，再生成摘要或高亮。
- 排序白名单、签名 keyset 游标、日期和版块过滤必须绑定查询上下文。
- Outbox 只负责缓存、Feed、分析和重建调度，不补写权威权限状态。
- 统一硬化阶段建立中文相关性样本和 10 万内容重建基准（`DEC-020`／`DEC-070`），责任人待 `DEC-011` 确定；未达标前不发布搜索。
- SQLite 体验模式：`search_documents` 只维护标题、可见性、状态与游标字段；标题检索优先 FTS5（启动探测），不可用时用小写规范化标题列 `LIKE`；搜索页明示“体验模式：仅支持标题搜索”。

## 14. 媒体与对象存储

上传状态机：`pending -> quarantine -> scanning -> ready`，失败进入 `rejected`，删除进入 `trashed -> purged`。

- 服务端验证扩展名、声明 MIME、真实 MIME、文件头、大小、像素和解码预算。
- 隔离对象不可公开下载。
- 恶意扫描、图片转码和元数据清除完成后才能发布。
- 对象 key 不包含用户名、邮箱或原文件名。
- 下载时重新校验 owner、引用、内容可见性和附件终态。
- 对象存储或扫描 provider 不可用时上传失败关闭，公开文本阅读继续。
- provider、费用、地域和保留期限待批准，不写生产默认。

## 15. 认证与会话

- 密码使用 Argon2id，长度 8 至 256 字符，可配置更高最小长度。
- 泄露密码检查通过可注入边界实现；DEC-051批准本轮版本化本地离线语料、无网络、启用不可用失败关闭，真实语料/外部provider仍属DEC-011/CR-014。
- 会话 token 和一次性 token 只存安全哈希。
- Cookie 为 `HttpOnly`、`SameSite`、生产 `Secure`，明确路径和有效期。
- 修改密码、邮箱、MFA 或账号状态时撤销相关会话。
- 普通用户会话绝对上限 30 天、滑动续期 14 天；管理会话最长 8 小时、空闲 15 分钟；高危重新认证 5 分钟内有效（`DEC-030`）。
- 一次性令牌有效期：邮箱验证 24 小时，密码重置 1 小时，均单次消费（`DEC-030`）。
- 特权角色强制 MFA；恢复码只显示一次并存哈希、单次消费。TOTP 共享密钥按 DEC-062 用 AEAD 加密后落库，只存密文、nonce 与密钥版本；包裹密钥取自版本化外部 keyring 并与邮件、风险密钥用途隔离，未注入时绑定与验签失败关闭，不退化为明文。密钥明文属 C3，永不由 /manage 返回，也不进日志、Outbox 与审计详情。
- DEC-051：预认证固定2小时并复用独立CSRF证明，新邮箱令牌24小时；同用户同用途的新签发与旧令牌失效同事务，重复幂等不再次失效。
- 一次性邮件令牌按版本化外部keyring、用途隔离HMAC和非秘密上下文重建；只存哈希、不存原始bearer，缺旧key保留原事件失败重试。邮件/风险密钥分离；Secret仅注入，不设通用默认。旧key按消息重放与备份恢复依赖退役，不从token有效期推导删除时间。
- 身份规范化、四维预算/网段、风险软阈值与本地挑战合同按DEC-051；公开错误和限流保持枚举防护，关闭Redis明示无保护，开启后失败按A05合同。

## 16. 审计与敏感数据

数据分级：

- C0：公开数据。
- C1：账号和普通操作元数据。
- C2：治理、风险、客服和受限证据。
- C3：密码、密钥、令牌、证件、原始敏感资料，永不由 `/manage` 返回。

高危写和敏感读记录操作日志，包含 actor、目标、动作、理由、工单、时间和结果，不记录敏感原文。

## 17. 可观测性

统计事件时间与接收时间使用UTC；统计时区固定UTC（+0，DEC-048），不随用户展示时区变化。MVP可信用户不设账号年龄门槛。DAU按自然日，WAU/MAU与果农指数按自然周/自然月，均为半开区间，日/月在00:00分界（DEC-049）。正文至少10、回复至少2个字符，按Unicode标量计数、1标量=1字符、不做宽度折算；自然周按ISO-8601从周一00:00 UTC开始（DEC-055）。浏览、有效回复、最后回复与首次阅读/首次回复的口径按DEC-054，见01 §4.5/4.18。其他有效性口径按01及已批准决策执行。

- 结构化日志带 request/trace/actor 摘要，不带 secret 或 C3。
- 指标覆盖 HTTP、数据库池、事务重试、Outbox lag、DLQ、邮件、媒体和搜索。
- tracing 贯穿 Web、事务、Outbox 和 sink。
- 健康检查区分 liveness、readiness 和依赖降级。
- 错误追踪 provider 待选，接入前使用本地脱敏接口并保持生产关闭。外部 provider 方向已按 `DEC-059` 定为托管服务优先（邮件、对象存储、验证码、错误追踪）；具体供应商、区域与隐私评估仍属 `DEC-011`，未选定前只交付 adapter 与故障语义。`pg_jieba` 搜索词典属自建扩展，不在托管方向内。

## 18. 故障语义

| 故障 | 必须行为 |
|---|---|
| 主库写不可用（三库通用语义） | 写请求返回受控 `503`，不写入内存假成功 |
| 主库读不可用（三库通用语义） | 不展示过期敏感状态；可按批准策略提供有限静态故障页 |
| Redis 不可用 | 公开读可降级；高风险限流依赖写失败关闭 |
| 对象存储/扫描不可用 | 附件写失败关闭，纯文本内容按原子边界处理 |
| SMTP 不可用 | 邮件保持 queued/failed，可重试，不伪造已发送 |
| 搜索不可用 | 搜索受控 `503`，公开主题直接阅读不受影响 |

### 18.1 备份与恢复

`PRIV-007`（删除墓碑在恢复后不得复活）与关键流程 5.6（账号安全与注销）都假设存在可用备份，但此前全套计划无备份条款。本节补齐规范，具体数值待 `DEC-004-B` 批准后填入 `91`：

- PostgreSQL 完整模式：每日逻辑备份 + WAL 归档到与应用不同的故障域；备份加密存储，保留期与访问责任人待批准。
- SQLite 体验模式：备份即复制数据库文件，必须在 `VACUUM INTO` 或停机状态下进行，直接拷贝 WAL 模式下的活动文件不可靠。
- 恢复演练：至少每个发布候选执行一次「从备份恢复到可服务」的完整演练并留存证据，未演练的备份不计入 Repository Ready。
- 恢复后的第一件事是重放删除墓碑与法律保留状态，再开放流量；否则已删除数据会随恢复复活，违反 `PRIV-007`。
- RPO/RTO 目标、备份保留期、异地副本与演练频率待 `DEC-004-B` 批准，计划不填猜测默认值。

## 19. 迁移策略

- 初始结构从空数据库建立（PostgreSQL 16 或 SQLite），不包含旧系统数据导入或兼容层。
- 迁移文件按序演进；修复通过新增迁移完成，不改写已应用迁移。
- 支持默认 `bcm_` 与非默认 schema 前缀（PostgreSQL）。
- 非本地数据库禁止自动清空。
- N 轨迁移集合与多集合编号对齐校验见第 6.3 节；执行入口 `bbbcm-cli db migrate --backend <postgres|sqlite>`（`mysql8` 为预留占位，未实现）。
