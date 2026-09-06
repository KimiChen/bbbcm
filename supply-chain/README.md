# 技术与供应链复现说明

本目录是 `WP-M0-03` 的配置与操作手册，计划、状态和批准仍只以
[`plan/README.md`](../plan/README.md)、[`02`](../plan/02-technical-architecture.md)、
[`90`](../plan/90-quality-release.md) 和 [`99` 第 5.2 节](../plan/99-traceability.md#52-wp-m0-03-技术与供应链基线2026-09-05) 为准。
M0-03 锁定构建前置；A01 直接共用本目录及根目录锁文件，交付实际 esbuild 管线、
`static/dist/` 内容哈希产物、manifest helper 与 immutable 路由。

## 1. 固定输入

版本与摘要唯一机器入口为 [`lock.json`](lock.json)。Cargo/npm 的传递依赖分别以根目录
`Cargo.lock` / `package-lock.json` 为准，不在本文件复制第二份依赖图。

| 输入 | 本轮取值与依据 |
|---|---|
| Rust / Cargo | 保留现有 1.97.0，rustfmt/Clippy 随相同工具链；官方 dist manifest 的 SHA-256 已登记。`rust-version = 1.88` 是既有最低版本声明，本轮没有运行 1.88 矩阵。 |
| Rust 库 | registry 包 335 个（原 326，`DEC-062` 为 TOTP 新增 `chacha20poly1305 0.10.1` 与 `sha1 0.10.7` 及其传递依赖），依赖图以根 `Cargo.lock` 为准；Axum 0.8.9、Askama 0.16.1、sqlx 0.8.6、Tokio 1.53.1。工作区 16 个成员。 |
| Node / npm | 24.20.0 LTS / 随附 npm 11.19.0；来自 Node 官方归档和 SHASUMS256。A00 全局 Node 26.5.0/npm 11.17.0 是采集环境盘点值，未成为构建基线，也未被本轮替换。 |
| esbuild | 精确 0.28.2；npm 官方 registry；共用 v3 锁文件含原生 optional 包，共 27 个包。安装时只选本机平台，锁文件保留所有平台。 |
| PostgreSQL | Docker Official Image `16.15-alpine3.23`；索引摘要、linux/amd64 与 linux/arm64 manifest/config 摘要分别登记。Alpine 版保留 server headers，可构建扩展。 |
| Redis | Docker Official Image `8.2.9-bookworm`，锁定已有维护分支的补丁版；同样登记两个平台，未为本包引入其他 8.x minor 功能。 |
| pg_jieba | 上游 control 版本 1.1.1，commit `d0ffac8028328b2566a889ff4db3d74ba63d1b42`；cppjieba、limonp 的递归 gitlink 与三个源码归档 SHA-256 已固定。 |
| pg_trgm | PostgreSQL 16.15 随附 1.6，control 文件 SHA-256 与所属 PostgreSQL 镜像绑定；不虚构独立扩展镜像。 |
| 编译包 | Alpine 3.23 每个平台 29 个 APK；完整包名、版本、SHA-256、依赖及 provides 均登记。GCC/g++ 15.2.0-r2、CMake 4.1.3-r0、make 4.4.1-r3。 |
| SQLite | sqlx 现有 `libsqlite3-sys 0.30.1` 的 bundled SQLite 3.46.0，不用宿主 `sqlite3` 或新增镜像替代；真实业务迁移尚待 A03。 |
| 辅助宿主 | Python 3.11.4+ 标准库（本轮 3.14.6）、Git、Rust 所需宿主 C 工具链；macOS arm64 已运行轻量检查。Linux 与 macOS x64 归档已锁定，未冒称执行过这些平台。 |

本轮适用的服务测试镜像仅 PostgreSQL 与 Redis。浏览器/视觉镜像随 E04/DEC-019 的批准环境登记，
SMTP、对象存储等 provider 测试环境随对应包明确；不填待决供应商的猜测镜像。

## 2. 干净检出与受控安装

已有 rustup 时，在仓库根目录执行：

```bash
rustup toolchain install 1.97.0 --profile minimal --component rustfmt --component clippy
cargo fetch --locked
./scripts/supply-chain.py install-tools node
./scripts/supply-chain.py npm-ci
./scripts/supply-chain.py check
./scripts/dev-check.sh
```

`rust-toolchain.toml` 自动选择 1.97.0，Rust dist 来源为 `static.rust-lang.org`。
Cargo 只允许 crates.io registry 和登记的 workspace 路径；`.cargo/config.toml` 固定 sparse 协议，
lock 中的 crates.io-index URL 是 Cargo 的逻辑来源标识，不表示使用另一套 registry。
`Cargo.toml` 的语义版本范围保留，实际解析由受版本管理的 `Cargo.lock` 与 `--locked` 固定。
来源和锁文件职责参考 [Cargo 官方说明](https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html)。

安装器仅下载 `lock.json` 的固定 HTTPS 地址，核对 SHA-256 后解包到 `.local/tools/`，
缓存写 `.cache/supply-chain/`。不改全局 Node/npm/PATH，不运行上游在线安装 shell。
Rustup 自身须来自 Rust 官方安装来源；本包不修改已有 rustup。
本轮是摘要校验，不声称额外完成了 GPG/Sigstore 发布者身份验签。

`npm-ci` 使用安装目录内的 Node 与 npm，忽略用户/全局 npm 配置并显式使用官方 registry；
`.npmrc` 固定 engine-strict、save-exact、ignore-scripts、无自动 audit、项目内 cache。
依赖生命周期脚本不执行，esbuild 通过锁定的 optional 原生包直接运行；不得省略 optional/dev 依赖。
这条路径已验证 esbuild 的内存 TypeScript transform，不生成 A01 的资产产物。
[npm ci](https://docs.npmjs.com/cli/v11/commands/npm-ci/) 会拒绝清单/锁文件不一致并保持锁文件不变；
平台包的约束见 [esbuild 安装说明](https://esbuild.github.io/getting-started/#install-esbuild)。

缓存准备后可重复执行：

```bash
./scripts/supply-chain.py install-tools node --offline
./scripts/supply-chain.py npm-ci --offline
cargo metadata --locked --offline --format-version=1 > .cache/cargo-metadata.json
```

离线缺包或摘要不符直接失败，不自动换镜像、升级或覆盖污染缓存。每个 OS/架构必须单独准备原生包缓存，
不能把 macOS node_modules 复制给 Linux。Cargo 的离线缓存包含 registry/index/src/cache 和所需工具链；
也可用 `cargo vendor --locked .cache/cargo-vendor` 配合其输出配置在隔离检出复现，vendor 不进公开仓库。
受控 Linux Rust 构建还需固定宿主/构建镜像的 C 编译器与链接器，E01 在实际候选中记录；本包不宣称跨宿主字节完全一致。

## 3. PostgreSQL 扩展测试环境

基础镜像按平台 digest 读取，禁止把 index 摘要、平台 manifest 摘要、config/image ID 混称：

```bash
./scripts/supply-chain.py image-ref --image postgres --platform linux/arm64
./scripts/supply-chain.py image-ref --image redis --platform linux/arm64
./scripts/supply-chain.py prepare-postgres --platform linux/arm64
```

`prepare-postgres` 校验 3 个源码包、29 个编译 APK，生成 `.cache/supply-chain/postgres/aarch64/`，
其中包含 Dockerfile、固定源码、APK、SHA256SUMS、lock.json 及 `build-command.txt`。
已有上下文会拒绝覆盖；需更新时先移走旧目录以保留旧输入。amd64 用 `--platform linux/amd64`，目录为 `x86_64`。
有完整缓存时添加 `--offline`，本轮两平台上下文均已这样准备。

在有 Docker/BuildKit 的本机测试宿主上，先拉取 `image-ref` 输出的基础镜像，再执行对应生成的构建命令。
命令固定平台摘要且 `--network=none`；Dockerfile 不访问 apt/apk 在线索引，APK 继续由基础镜像自带 Alpine keys 校验。
CMake 的 `CMAKE_POLICY_VERSION_MINIMUM=3.5` 用来兼容上游旧 minimum 声明，不修改上游源码。
运行层保留固定的 musl/libgcc/libstdc++、pg_jieba 和上游许可证；原镜像 pg_trgm 不另编一份。

```bash
docker pull --platform linux/arm64 "$(./scripts/supply-chain.py image-ref --image postgres --platform linux/arm64)"
sh .cache/supply-chain/postgres/aarch64/build-command.txt
docker image inspect bbbcm-postgres:m0-03-aarch64
docker save --output .cache/supply-chain/postgres/aarch64/image.tar bbbcm-postgres:m0-03-aarch64
```

pg_jieba 没有在本包采用的官方成品镜像；它的锁定单位是基础镜像 manifest + 源码/子模块归档 + APK + Dockerfile。
生成镜像的 `image-id.txt` 是实际构建后的 config ID，不能冒充 registry manifest digest；没有构建就不填产物摘要。
E01 留存 `docker save` 归档 SHA-256、image inspect、OS/架构、构建日志和扩展安装/运行结果，并检查可重复构建。
当前宿主无 Docker，未执行上述构建、加载或服务集成。

测试服务只在本机临时容器/卷中运行：使用随机临时凭据或 Docker secret 文件，不写入仓库；仅需容器内测试时
使用 `--network=none`，需要宿主访问时端口仅绑定 `127.0.0.1`。PostgreSQL 必须 UTF-8、非 C/POSIX lc_ctype；
空测试库中确认 pg_jieba=1.1.1、pg_trgm=1.6 及中文分词后才用于 E01。测试词典只是锁定 cppjieba 自带词典，
没有批准生产词典与责任人。Redis 只用于 `bcm:test:v1:` 下的可重建测试状态；没有生产配置或清库命令。
本轮不初始化业务数据库、Schema、迁移、权限或 Outbox。

Alpine 的在线 APKINDEX 会变化，因此只用它做版本盘点，复现用锁定的 APK URL 与字节摘要。
上游包被移除时从已留存的受控归档恢复同摘要文件，不能重新解析“当前可用”包；缓存应随硬化候选留存。

## 4. 扫描规则与独立入口

| 范围 | 工具/配置 | 规则 |
|---|---|---|
| SBOM | Syft 1.51.1、`syft.yaml`；npm 11.19.0 自带 SBOM | CycloneDX + Syft JSON；覆盖 Cargo、npm dev/optional 所有平台、pg_jieba/cppjieba/limonp 源码及实际容器 OS 包；扫描结果必须核对锁文件覆盖。 |
| 许可证 | Trivy 0.74.0、`trivy.yaml`、license inventory、A00 原清单 | Cargo 元数据补全许可证表达式；未知字段显式留空并列为 unknown-needs-review；npm 和 C++ 的许可证不省略。扫描器的风险分类不是法律授权结论。 |
| 漏洞 | Trivy 0.74.0、`trivy.yaml`、空 `trivy.ignore` | 保存所有严重度及无修复项，Critical/High 返回失败；Medium/Low/Unknown 全部留在报告中，交 E01 登记与定期处置。 |
| Secret | Gitleaks 8.30.1、`gitleaks.toml`、空 `gitleaks.ignore` | 启用全部内置规则、100% redact；全历史或显式暂存区范围；不接受内联 allow 注释，不给业务/证据目录加白名单。 |

工具来自各自官方 GitHub release，归档 SHA-256 与官方 checksums 和 release asset digest 双向核对；
本地二进制还检查版本。没有运行浮动 GitHub Action，没有 npx 在线自动选版。
配置与脚本以本提交版本锁定，依赖和工具的上游许可证可从对应归档取回；不把工具自身声明覆盖到扫描对象上。

先安装工具，然后只在 **E01** 的干净已提交候选上运行全量入口；每次使用新的报告目录：

```bash
./scripts/supply-chain.py install-tools
cargo fetch --locked
./scripts/supply-chain.py npm-ci
./scripts/supply-chain-scan.py sbom --output .cache/supply-chain/reports/e01-source-01
./scripts/supply-chain-scan.py licenses --sbom .cache/supply-chain/reports/e01-source-01/source.cdx.json --output .cache/supply-chain/reports/e01-licenses-01
./scripts/supply-chain-scan.py fetch-db --output .cache/supply-chain/reports/e01-db-01
./scripts/supply-chain-scan.py vulns --sbom .cache/supply-chain/reports/e01-source-01/source.cdx.json --output .cache/supply-chain/reports/e01-vulns-01
./scripts/supply-chain-scan.py secrets --output .cache/supply-chain/reports/e01-secrets-01
./scripts/supply-chain-scan.py image-sbom --archive .cache/supply-chain/postgres/aarch64/image.tar --output .cache/supply-chain/reports/e01-postgres-01
```

对 PostgreSQL/Redis 的实际镜像归档各执行 image-sbom，再将其 image.cdx.json 分别传入 licenses/vulns。
源码 SBOM 和镜像 SBOM 一并归档，pg_trgm 按镜像所属 PostgreSQL 版本及实际 `pg_extension` 输出核对；
不得把仅有源码包清单当实际容器已通过扫描。二进制候选与部署构建的 SBOM 随 A01/E01 实际产物补齐。

漏洞库是会更新的情报，不永久锁一份旧数据库：`trivy-db:2` 是数据库 schema 通道，只有显式 fetch-db 才访问；
vulns 仅离线读取固定快照，并记录 trivy.db/metadata.json 的 SHA-256、更新时间，确认扫描期间未变化。
E01 每个新候选先刷新并留存这两份文件；复核旧候选时恢复同摘要快照，而非从当前通道重新下载。
缺库、下载错误、扫描错误均失败，不能记作“零漏洞”。

许可证入口产生待审核清单，返回 0 只说明扫描执行完成。项目自有 AGPL-3.0-only 属既定许可证；
Trivy 默认分类可能把它列为 Forbidden，须在法律审核中区分项目自身授权与第三方义务，不能据该分类自行改许可证。
分发形式、署名/NOTICE、源码提供义务、组合表达式以及未知条目须逐项复核；缺许可证不能凭 SBOM 成功变成已授权。
A00 的 `assets.json` 原样附带，DEC-045/046 的未知许可与本机版本化口径保持，**不作为本轮完成/提交的阻塞**；
对外发布的法律处理仍按原计划，不在本包新增批准门禁。

Critical/High 清零、正式 SBOM/Secret/许可证/漏洞全量证据仅属于 E01；本包不提前检查全量清零，
不授予 Verified/M0 Repository Ready。公开测试前 Critical/High 为零，例外/Medium/Low 处置按 `90` §8，
负责人、到期和关闭条件登记在 `99`，不从报告删除问题。

所有原始报告只落 `.cache/supply-chain/`，失败也生成 run.json（候选 commit、输入/报告摘要、时间与退出码）；
原始报告可能含文件路径或敏感上下文，不能直接提交。E01 脱敏后转存 `90` 规定的候选证据目录并在 `99` 登记。

## 5. 开发期检查与更新

本轮及以后修改本基线时运行：

```bash
./scripts/supply-chain.py check
python3 scripts/test_supply_chain.py
./scripts/dev-check.sh
git diff --check
```

前两项检查锁文件、摘要、依赖闭包及合成小夹具的工具行为，不扫描完整项目或下载漏洞库；
不无条件接入每次 dev-check，更不加入独立 cargo build/esbuild 完整资产构建（DEC-039）。
暂存后可做本轮变更范围的 Secret 检查：

```bash
./scripts/supply-chain-scan.py secrets --staged --output .cache/supply-chain/reports/dev-staged-01
git diff --cached --check
git diff --cached
```

更新由项目负责人最终批准，Codex 可在授权工作包内实施：先明确安全/兼容/功能原因，查官方发布说明与 MSRV、
上游仓库及许可证，再定点更新。无必要不升级已有可用版本；不批量 `cargo update`，不自动 `npm audit fix`。
Rust 库用 `cargo update -p <包> --precise <版本>`；Node/npm 必须配套更新官方归档摘要和 engines/packageManager；
esbuild 修改 package.json 后用 `./scripts/supply-chain.py npm-lock` 重算唯一根锁文件，再 npm-ci 复核。
不要删除 Cargo.lock/package-lock.json 来“修复”安装失败。

镜像升级重新核实标签、索引、平台 manifest、config 的对应关系；源码升级递归核对 gitlink 和每个归档摘要；
APK 逐包核对包内 .PKGINFO、依赖和实际字节摘要，不仅锁四个顶层编译包。所有变化连同许可证、漏洞影响、
检查结果及 E01 证据失效范围写入 `99`。本机版本化后仍不自动 push、部署或启用 Feature Flag。
