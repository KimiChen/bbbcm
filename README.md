# 草莓社区（bbbcm）

开源中文技术社区软件：Rust + Axum + Askama SSR。支持 PostgreSQL（完整模式）与 SQLite（体验模式，零外部依赖）双后端；SQL 按 PostgreSQL/SQLite/MySQL 8.0 三库交集子集编写，预留 MySQL 8.0（规范先行，暂未实现）。Redis按架构为可选缓存、短租约、唤醒与限流组件；目前只做配置校验，运行时接入归A05。

> **开发中。** 已提供共用配置、PostgreSQL/SQLite 迁移、状态查询和开发种子。已增加事务/幂等、Outbox/Worker、追加审计与 Flag 数据库只读查询；身份、内容、后台业务及 Flag 受控写仍待后续交付，所有业务 Flag 默认关闭。状态以 [`plan/99-traceability.md`](plan/99-traceability.md) 为准。

## 快速体验（SQLite）

```bash
cargo run --locked --package bbbcm-cli -- db migrate --backend sqlite
cargo run --locked --package bbbcm-cli -- db status
cargo run --locked --package bbbcm-cli -- db seed
cargo run --locked --package bbbcm-web  # http://127.0.0.1:8080
```

体验模式默认 `features.redis = false`，因此**不提供限流**：注册、登录、找回密码没有速率保护。该模式用于本机体验与评估，不要公开部署（`DEC-024`）。公开部署须等后续实现和发布门禁；当前设 `features.redis = true` 不会获得尚未实现的生产缓存或限流能力。

### 环境变量

| 变量 | 作用 |
|---|---|
| `BBBCM_CONFIG` | 配置文件路径，默认 `config/default.toml` |
| `BBBCM_ASSET_DIST` | 指纹资源目录；默认 `apps/web/static/dist`，显式指定时缺失或损坏必须拒绝启动 |
| `BBBCM_DATABASE_POSTGRES_URL` | PostgreSQL 连接串。只从环境变量注入，禁止写入任何配置文件 |
| `BBBCM_REDIS_URL` | Redis 连接串；启用 Redis 时必需，只经 Secret 注入 |
| `RUST_LOG` | 日志级别，默认 `info` |

共用加载优先级为安全默认、主配置文件、本机 `config/local.toml`、环境变量、Secret 注入。四环境模板、完整变量映射、PostgreSQL 角色初始化、迁移/种子失败处理和本机清理见 [数据库手册](docs/development/database.md)。

配置文件的未知键会导致启动失败而非被静默忽略；`server.environment` 只接受 `development`、`test`、`staging`、`production`。

## 进程

| 程序 | 作用 |
|---|---|
| `bbbcm-web` | Web 服务（Axum + Askama SSR） |
| `bbbcm-worker` | 持久化Outbox轮询、本地sink、关闭的审计副本；体验模式可内嵌web。邮件/媒体/清理待后续 |
| `bbbcm-cli` | 迁移、状态、开发种子与Flag数据库查询；账号引导/Flag写入待后续 |

事务调用、未知提交查询、Worker/DLQ恢复与Flag查询见 [基础引擎手册](docs/development/engine.md)。

## 代码结构

- `apps/`：三个可执行程序（web、bbbcm-worker、bbbcm-cli）。
- `crates/`：平台库（platform、platform-db、platform-outbox、platform-audit）、领域库（identity、content、media、interaction、moderation、support、trust、privacy）与管理后台库（manage）。
- 每个领域 crate 内部固定四层：`contract.rs`（类型契约）/ `core.rs`（业务服务）/ `repo.rs`（数据访问）/ `http.rs`（路由）。

设计与计划见 [`plan/README.md`](plan/README.md)——它是唯一计划入口与事实源。历史设计稿存于 `docs/design-notes/`，仅供追溯，不具事实源效力。

## 开发

提交前跑一次开发期最小集（格式化、Clippy、单元测试、SQLite 冒烟、暂存区复核与静态守卫）：

```bash
./scripts/dev-check.sh
```

也可以单独执行其中任一步：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

工具链由 `rust-toolchain.toml` 钉定，`rustup` 会自动切换。

Node/npm/esbuild、服务测试镜像、供应链扫描与离线缓存的受控复现入口见
[`supply-chain/README.md`](supply-chain/README.md)。A01 共用根目录 npm 锁文件；执行
`./scripts/assets.py build` 生成指纹资源，重启 web 后现有 SSR 使用 manifest helper。
构建、测试、运行、错误处理与清理见 [静态资产手册](docs/development/assets.md)。
开发/测试的默认 dist 从未生成时保留明确提示的 A00 源模式；staging/production 必须有有效 dist。
完整资产构建不无条件加入每次 dev-check（DEC-039）。

## 许可证

本项目采用 [GNU AGPL-3.0-only](LICENSE)。许可证适用于本项目自有代码、迁移、配置模板与 `plan/` 文档。

仓库中的品牌图形、图标与字体等静态资产各有独立来源与许可，逐项登记在资产清单（见 `plan/05-visual-brand-spec.md` 第 4 节）。本机版本化遵守 DEC-045/046，未知许可如实记录，不作为本轮开发或本机提交的阻塞；不据此宣称已取得第三方授权，对外发布前仍须处理相应权利。
