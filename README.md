# 草莓社区（bbbcm）

开源中文技术社区软件：Rust + Axum + Askama SSR。支持 PostgreSQL（完整模式）与 SQLite（体验模式，零外部依赖）双后端；SQL 按 PostgreSQL/SQLite/MySQL 8.0 三库交集子集编写，预留 MySQL 8.0（规范先行，暂未实现）。Redis 可选，承载缓存、短租约、唤醒与限流。

> **开发中。** 当前仓库处于骨架阶段：迁移执行器、种子数据与业务路由尚未接入，下方“快速体验”的 `db migrate`、`db seed` 会以退出码 1 明示“尚未实现”。工作包状态以 [`plan/99-traceability.md`](plan/99-traceability.md) 为准。

## 快速体验（SQLite）

```bash
cargo build --release
./target/release/bbbcm-cli db migrate --backend sqlite   # 随 WP-MVP-A03 接入
./target/release/bbbcm-cli db seed                       # 随 WP-MVP-A03 接入
./target/release/bbbcm-web                               # http://127.0.0.1:8080
```

体验模式默认 `features.redis = false`，因此**不提供限流**：注册、登录、找回密码没有速率保护。该模式用于本机体验与评估，不要公开部署（`DEC-024`）。公开部署请使用完整模式：PostgreSQL + `features.redis = true`。

### 环境变量

| 变量 | 作用 |
|---|---|
| `BBBCM_CONFIG` | 配置文件路径，默认 `config/default.toml` |
| `BBBCM_DATABASE_POSTGRES_URL` | PostgreSQL 连接串。只从环境变量注入，禁止写入任何配置文件 |
| `RUST_LOG` | 日志级别，默认 `info` |

配置文件的未知键会导致启动失败而非被静默忽略；`server.environment` 只接受 `development`、`test`、`staging`、`production`。

## 进程

| 程序 | 作用 |
|---|---|
| `bbbcm-web` | Web 服务（Axum + Askama SSR） |
| `bbbcm-worker` | 后台任务（Outbox、邮件、媒体、清理）；体验模式可内嵌进 web（`worker.embedded = true`） |
| `bbbcm-cli` | 迁移、种子数据、账号引导、Feature Flag 管理 |

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
[`supply-chain/README.md`](supply-chain/README.md)。A01 共用根目录 npm 锁文件，资产构建管线仍待 A01 接入。

## 许可证

本项目采用 [GNU AGPL-3.0-only](LICENSE)（`DEC-023`）。许可证适用于本项目自有代码、迁移、配置模板与 `plan/` 文档。

仓库中的品牌图形、图标与字体等静态资产各有独立来源与许可，逐项登记在资产清单（见 `plan/05-visual-brand-spec.md` 第 4 节）；未确认可再分发的第三方资产不随本仓库分发。
