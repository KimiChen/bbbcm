# WP-M0-03 开发期证据（2026-09-05）

代码/配置版本为本记录所在的 `新增：完成 WP-M0-03 技术与供应链基线` 提交。
开始时 HEAD 为 `2d3df36`、工作区干净；用户并行提交 `6a93847` 已保留，
其中包含本轮实施前 `99` 登记。项目负责人/最终批准人是用户，Codex 负责实现、验证与本机提交。
M0-03 本次状态为 `Implemented`；输入和开发期检查摘要见 [`validation.json`](validation.json)。

## 交付与范围

- 保留 Rust/Cargo 1.97.0、16 成员 Workspace、247 个 registry crate、Cargo.lock（SHA-256 为 `48d13360ca82623dfacd50c1f85ff4310c0f7d5ab2053de98436126dd7e725a4`）；没有版本升级。
- 根目录 Node 24.20.0/npm 11.19.0/esbuild 0.28.2、v3 package-lock.json（27 个 npm 包）与受控/离线安装。
- PostgreSQL 16.15-alpine3.23、Redis 8.2.9-bookworm 两平台索引/manifest/config 摘要；pg_jieba 1.1.1、cppjieba、limonp 源码 commit/归档 SHA-256；pg_trgm 1.6 随 PostgreSQL；58 个 APK 的完整依赖闭包和字节摘要。
- Syft 1.51.1、Trivy 0.74.0、Gitleaks 8.30.1 的版本、官方归档摘要、配置、独立入口和操作手册。
- `supply-chain/README.md` 是配置操作说明，计划和状态仍在 plan/。A01 的实际资产管线尚未实现；本轮没有改 A00 模板/CSS/资产、应用源代码、迁移、权限、Outbox 或 Feature Flag。

## 开发期检查

| 命令/检查 | 结果与证据 |
|---|---|
| `./scripts/supply-chain.py check` | 通过，见 [`baseline-check.txt`](baseline-check.txt)；16 成员、247 个 registry crate、27 个 npm 包、77 个固定归档。Cargo/npm 来源、版本、SHA、镜像平台及 APK 依赖闭包一致。 |
| `install-tools` | 本机 darwin/arm64 的 Node/Syft/Trivy/Gitleaks 归档与固定 SHA-256 一致，解包后版本一致。其他平台只核实官方归档摘要，未执行二进制。 |
| `npm-lock`、`npm-ci`、`npm-ci --offline` | 使用官方 Node 归档内 npm 生成唯一锁文件；受控安装及缓存离线重装均通过，npm ci 前后锁文件未变。esbuild 原生版本和内存 TS transform 已确认，无资产输出。 |
| 官方镜像及 gitlink 核实 | [`upstream.json`](upstream.json)；两个 index、四个 manifest、四个 config 的实际响应字节 SHA-256 与锁定值匹配；OS/架构/版本环境变量一致；三层 gitlink 闭包匹配且 limonp 无额外子模块。 |
| 源码与 APK | 3 个源码包和 58 个 APK 实际下载并核对 SHA-256；APK 内 .PKGINFO 的包名、版本、架构、depends/provides 与锁文件一致。源码 LICENSE 内容哈希已登记。 |
| `prepare-postgres --platform linux/arm64 --offline` 和 amd64 | 通过；使用完整校验缓存生成两个当前版本构建上下文，SHA256SUMS 含源码、编译包及运行包，构建命令只使用平台 digest 和禁网 RUN。没有执行 Docker。 |
| `python3 scripts/test_supply_chain.py` | 10/10 通过，最新日志 [`light-tests-04.txt`](light-tests-04.txt)。覆盖缓存污染、离线缺包、tar 路径穿越、未知许可保留、Critical/High 判定、npm optional/C++ 清单及包名/版本逐项覆盖、原生 esbuild、Syft/npm SBOM 和 Trivy 许可合成夹具、缺漏洞库失败、Gitleaks 命中与脱敏。 |
| `./scripts/dev-check.sh` | 通过；fmt、Clippy -D warnings、65 个非零 Workspace 单元测试、48 条 SQLite 配置下 HTTP 冒烟、diff、暂存区列表和 handler 静态守卫。SQLite 迁移执行器未接入，脚本明确跳过；不能记作真实迁移验证。初次日志 [`dev-check-01.txt`](dev-check-01.txt)，最新日志 [`dev-check-02.txt`](dev-check-02.txt)。 |
| `git diff --check`、`git diff --cached --check` | 均通过。 |
| 暂存区逐文件复核与 Secret | 通过；逐文件检查范围、差异、UTF-8/LF/末尾换行及生产/个人信息脱敏，Gitleaks 暂存区扫描无发现。开发期扫描输入/结果摘要见 validation.json；只检查本轮暂存文件，不替代全仓库历史扫描。 |

## 已修正的开发失败

- 最初沙箱 DNS 拒绝官方查询/归档下载，经批准访问后完成；没有用猜测版本或摘要填补失败。
- npm 11 拒绝 user/global 配置同时指向 `/dev/null`，入口改为独立空 global 配置后成功；首次失败未记作安装成功。
- 第一版一致性检查把 PostgreSQL 的两段版本误当三段 SemVer，已按 PostgreSQL 16.x 与 Redis 8.x.x 分别校验。
- 第一次 Secret 合成夹具使用连续字母/数字，被默认规则识别为明显示例而忽略；保留失败日志 [`light-tests-01.txt`](light-tests-01.txt)，改用哈希生成的无效测试 token 后检测和100%脱敏通过，见 [`light-tests-02.txt`](light-tests-02.txt)。没有放宽检测规则或断言。
- 初次暂存的脱敏 `.log` 被既有忽略规则拒绝，已改用 `.txt` 归档并正常暂存，没有强制添加忽略文件。

## 未执行与后续边界

本机没有 Docker，所以没有编译/加载派生 pg_jieba 镜像、没有镜像产物 digest、没有真实 PostgreSQL/Redis 集成。
已锁定的是可复现输入与构建配方；实际 image ID、归档 SHA-256、扩展版本/中文行为、跨宿主重复构建留 E01。
本轮没有运行正式全仓库/全镜像 SBOM、许可证、漏洞、Secret 历史扫描或下载漏洞数据库，
没有作 Critical/High 清零、供应链风险完全排除、Verified 或任何 M0/发布门禁通过的结论。

A01 的 esbuild 管线、static/dist 内容哈希、manifest helper 和 immutable 路由仍由 A01 交付。
A00 未知许可保持原登记及 DEC-045/046 本机版本化口径，不恢复被取消的提交前置。
本轮仅本机提交，不部署、不推送。
