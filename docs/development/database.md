# 本机配置与数据库初始化

本手册是 [计划入口](../../plan/README.md) 下数据库基础实现的操作说明；状态和证据统一见 [99 §5.4](../../plan/99-traceability.md)。数据库基础、四版块骨架和规则占位沿用M0-04/A03；本轮新增事务/Outbox/审计及Flag只读存储的接线与恢复见 [基础引擎手册](engine.md) 和99 §5.5。身份、内容业务、管理后台及Flag受控写仍待后续。

## 配置加载

三个可执行都调用同一个 `Config::from_environment`，优先级从低到高：Rust 安全默认 → `BBBCM_CONFIG` 指定文件（缺省 `config/default.toml`）→ 当前工作目录下可选 `config/local.toml` → 环境变量 → Secret 注入。每个文件层先检查未知键和类型，再合并；拼错的低优先级键不能被后层覆盖隐藏。显式主文件缺失拒绝启动，本机覆盖不存在可忽略。相对文件路径以进程工作目录为准。

`Config::load_layers` 是可测试的共用注入接口，调用者显式传入主文件、本机文件、环境映射及 `SecretOverrides`；不修改进程全局环境。`Config::load` 仅解析单文件，启动应使用完整入口。

| 环境变量 | 类型 / 含义 |
|---|---|
| BBBCM_CONFIG | 必需存在的主 TOML 路径 |
| BBBCM_SERVER_ENVIRONMENT | development / test / staging / production |
| BBBCM_SERVER_BIND | IP:端口，SQLite 和 staging/production 仅回环地址 |
| BBBCM_DATABASE_BACKEND | postgres / sqlite；mysql8 预留且拒绝启动 |
| BBBCM_DATABASE_SCHEMA_PREFIX | 受信前缀，默认 bcm_；production 固定 bcm_ |
| BBBCM_DATABASE_SQLITE_PATH | 普通持久化文件路径，SQLite 单写者 |
| BBBCM_DATABASE_MAX_CONNECTIONS | 正整数；SQLite 必须为 1 |
| BBBCM_FEATURES_REDIS | true / false；关闭时不提供限流 |
| BBBCM_WORKER_EMBEDDED | true / false；SQLite 的独立 Worker 与 true 互斥 |
| BBBCM_DATABASE_POSTGRES_URL | Secret，PostgreSQL 连接串，只经最终环境/Secret 层注入 |
| BBBCM_REDIS_URL | Secret；Redis 启用时必需，只经最终环境/Secret 层注入 |
| BBBCM_ASSET_DIST | A01 资产目录，由资产组件独立校验 |

未知 BBBCM_* 变量、未知配置键、非法类型、占位符或非法连接参数均拒绝。PostgreSQL URL query 仅允许 TLS 选项、application_name 和 statement-cache-capacity；host/user/password/database/port 只写 authority/path，不能用 query 覆盖。staging/production PostgreSQL 必须显式要求 TLS（sslmode=require/verify-ca/verify-full），拒绝驱动默认的可明文降级模式。Redis URL 当前不接受 query。SQLite 路径拒绝 file: URI 和内存库。不将 PostgreSQL/Redis URL 写入 TOML、命令参数、Git、日志或证据。Secret 的 Debug 固定脱敏；底层连接错误不会原文输出。注入真实连接值应使用本机受控进程环境或密钥管理工具，避免 shell 历史。拒绝 PG* 驱动环境变量旁路，不读取默认或显式 .pgpass；将参数统一移入受控 BBBCM 配置/Secret 层。

`config/{development,test,staging,production}.toml.example` 是脱敏模板。development/test 可直接用于本机体验；staging/production 模板的空 bind 和零连接上限是故意的非法哨兵，必须注入实际已批准值和数据库/Redis Secret 后才能启动，不能把模板当作已批准部署参数。provider 尚未批准和实现，没有虚构 provider 默认配置。

完整模式 Web/Worker 启动探测 PostgreSQL locale、数据库角色及扩展；不接受非 UTF8 或 C/POSIX 的 lc_ctype，缺 pg_jieba/pg_trgm 则明确提示，不会启用搜索能力或自动改写数据库Flag。Redis 当前仅校验配置，连接、限流/缓存运行时属于 A05；现有业务 Flag 全关。SQLite 夹具仍不依赖业务池。A00/A01 的 staging/production 夹具和源资产拒绝规则不变。

## SQLite 本机初始化

在仓库根目录执行，复用现有工具链；`cargo run` 是命令执行，不增加独立构建步骤：

```bash
cargo run --locked --package bbbcm-cli -- db status
cargo run --locked --package bbbcm-cli -- db migrate --backend sqlite
cargo run --locked --package bbbcm-cli -- db status
cargo run --locked --package bbbcm-cli -- db seed
cargo run --locked --package bbbcm-web
```

默认数据库为 Git 忽略的 `data/bbbcm.db`。`db status` 不创建数据库或表，空库显示 pending；`migrate` 使用独立单连接、WAL、5000ms busy timeout、外键 ON。重复 migrate 跳过已有版本，修改已应用文件则明确拒绝。

种子仅 development/test：四个 draft 版块“问答/实战”“AI”“域名与主机”“站务”，各有一个 placeholder 规则版本。按稳定键缺失才插入，重复执行保留已有编辑；不创建账号、SiteOwner 或权限 registry，也不生成真实种子文章/生产政策。staging/production 在连接前拒绝 seed。

配置示例可复制到 `config/local.toml` 后编辑非秘密值；该文件默认忽略。隔离测试应在独立临时工作目录运行，BBBCM_CONFIG 使用绝对路径，迁移目录也显式给出，防止仓库 local 覆盖影响临时配置：

```text
bbbcm-cli db --migrations-dir /absolute/repository/db/migrations migrate --backend sqlite
bbbcm-cli db --migrations-dir /absolute/repository/db/migrations status
bbbcm-cli db --migrations-dir /absolute/repository/db/migrations seed
```

独立 `bbbcm-worker` 在 SQLite 默认 `worker.embedded=true` 时拒绝启动。需要独立宿主时所有进程共用 `worker.embedded=false` 配置；现已接同一持久化Outbox调度器，并用OS锁阻止两个SQLite宿主同时运行；详见 [基础引擎手册](engine.md)。

## PostgreSQL 角色与初始化

本轮提供可执行 SQL 和 runner，不执行真实 PostgreSQL/Redis 环境矩阵。完整模式使用预先创建、UTF8 且非 C/POSIX locale 的 PostgreSQL 16 隔离数据库。扩展按已有 [供应链手册](../../supply-chain/README.md) 准备，不从未知网络源临时安装。

先在本机配置选择所需受信前缀，再输出一次性角色脚本：

```bash
mkdir -p .local
cargo run --locked --package bbbcm-cli -- db bootstrap-sql > .local/database-roles.sql
```

输出没有 Secret；用具备角色管理权限的管理员在**新的隔离数据库**执行该脚本，psql 必须设置 `ON_ERROR_STOP=1`。脚本创建前缀化 NOLOGIN/NOINHERIT 组 migrator/app_rw/worker_rw/report_ro，限制数据库和 public schema 的公共创建权限，仅给 migrator 数据库 CREATE。存在同名角色时脚本失败回滚，不自动接管/修改已有角色。它不适用于未经评审的共用数据库全局 ACL 调整。

真实 LOGIN、认证/TLS、数据库连接及角色成员关系由数据库管理员配置，Secret 不写进本脚本。迁移登录只获 migrator 的 `SET TRUE, INHERIT FALSE, ADMIN FALSE`；Web/Worker 的 LOGIN 本身必须 NOINHERIT，唯一成员关系为对应 runtime 组的 `SET TRUE, INHERIT FALSE, ADMIN FALSE`；不属于 migrator，不持有其他高权组、直接对象授权、数据库/schema 所有权或 CREATE/TEMP 权限。PostgreSQL 16 的角色成员具有独立 SET/INHERIT 选项，不能仅凭组的 NOINHERIT 推断安全。[PG16 角色成员说明](https://www.postgresql.org/docs/16/role-membership.html)

runner 连接后校验四组角色属性与成员关系，再 SET ROLE 到 migrator；Web/Worker 检查并切换相应组。对象 owner 统一为 migrator。管理员配置完成且 Secret 注入当前进程后：

```bash
cargo run --locked --package bbbcm-cli -- db migrate --backend postgres
cargo run --locked --package bbbcm-cli -- db status
```

`0001` 初始迁移创建15个schema，当时的表为schema_migrations、forums、rule_versions。每表/序列显式撤权；app_rw 只获两张骨架表 SELECT 和所需 schema USAGE，worker/report 暂无表权限。这是0001的授权边界；0002引擎和0003 Flag另加显式ACL，审计仅追加，Flag运行角色只读，IAM仍无业务表或授权。每次新建表必须在同一迁移追加 ACL，未来审计历史不得授予 runtime UPDATE/DELETE/TRUNCATE。

migrator 的未来对象默认权限同时在全局和各 schema 撤销 PUBLIC/runtime 授权，包括 TABLES/SEQUENCES/FUNCTIONS/TYPES。只做 schema 级撤销不能抵消全局默认授予，因此两层同时设置。[PG16 默认权限说明](https://www.postgresql.org/docs/16/sql-alterdefaultprivileges.html)

## 迁移编写、状态与故障处理

两条有效轨文件名从 `0001_init.sql` 按编号有序对齐；mysql8 保留空占位，不能借空轨豁免 PostgreSQL/SQLite。不改已应用迁移，以新增编号前向修复。迁移完整 UTF8/LF 字节参与 SHA-256；名称、编号、内容与历史不符均拒绝。

SQL 使用受信模板，例如 `{{table:community.forums}}`、`{{schema:community}}`、`{{role:app_rw}}`、`{{sequence:community.forums.id}}`。SQLite 替代表使用 `{{rebuild:community.forums}}`，由已注册表派生固定 `_rebuild` 后缀并校验长度；用于复制/删旧/改名的真实重建路径。后端相关 schema/role/sequence 模板仅用于 PostgreSQL；未登记对象无法渲染。新表必须同步 `SchemaRenderer` 注册，禁止任意 SQL 字符串输入、物理前缀硬编码或业务值插值。

每个迁移在独立事务内：PostgreSQL 取得包含前缀的 advisory transaction lock；SQLite BEGIN IMMEDIATE。SQLite 重建文件第一行为 `-- rebuild`，runner 在事务外关闭外键，提交前 foreign_key_check，成功/失败退出恢复 ON；迁移源不得自行提交或切换 PRAGMA。连接取消/关闭后不得把关闭外键的连接回收进业务池。

状态分 pending、applied、failed。结构和 applied 元数据同事务；确定失败先回滚并恢复外键，再独立锁事务记录安全 failed 状态。同一内容可重试，内容漂移拒绝。提交结果未知时命令失败并要求重新 status 核验，不声称已回滚，也不写假 failed 覆盖潜在成功。元数据损坏、缺文件、顺序错误须人工定位；不提供清空/改 checksum 的“修复”命令。

共享 SQL lint 随 `cargo test --workspace --locked` 扫描实际 repo SQL，允许 CTE/窗口函数，拒绝 02 §6.1/DEC-033 的语法与语义禁令。LIKE 必须作用于 `_normalized` 列并带 ESCAPE；标识符、列非空和规范化过程仍需代码复核。DDL 迁移另检查全局禁令，不以它为业务 SQL 方言逃生口。

## 本机清理与验证

`./scripts/dev-check.sh` 已接真实临时 SQLite 迁移，任何迁移失败立即中止；临时数据库和进程在退出时清理，不再接受“尚未实现”跳过。单元测试使用独立临时目录，覆盖重复、漂移、失败回滚、FK恢复、种子和实际 CLI。完整资产构建不加入该步骤。

停止本机 web/worker 与数据库连接后，确认配置指向本机测试文件，再删除该文件及其同名 `-wal`/`-shm` 伴随文件；保留所需数据前先备份，活动 WAL 数据库不能直接复制主文件。SQLite 备份使用停机复制或 VACUUM INTO（02 §18.1）。测试临时目录由测试清理；不自动清空非本地数据库，不执行 DROP DATABASE 或 Redis FLUSHDB。生成的 `.local/database-roles.sql` 用后可删除，它不是新计划或 Secret 文件。

本机 data、local 配置、数据库及 WAL/SHM、缓存和临时产物保持 Git 忽略。真实 PostgreSQL 双前缀/未来对象 ACL、Redis、安全、并发、跨平台、性能和视觉完整矩阵留 E01/E02/E04；本机 SQLite 通过不等于这些检查通过。
