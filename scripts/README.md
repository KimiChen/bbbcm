# 可重复验证脚本

存放可重复执行的验证脚本（格式化+静态检查+测试的组合入口等）。
按 DEC-020 分两层提供入口：开发期最小集（fmt + Clippy `-D warnings` + 单元测试 + SQLite 冒烟）与统一硬化阶段完整验证。
脚本必须可在干净检出上运行，不依赖本机状态。

`dev-check.sh` 的 SQLite/HTTP 冒烟通过 `cargo run --locked` 运行当前源码，避免复用
Clippy 或测试未刷新的普通可执行文件；它仍是七步开发期检查，不增加独立构建步骤（DEC-039）。
迁移通过专用连接实际执行，任何失败都会中止；隔离工作目录与 BBBCM 环境变量，防止本机配置覆盖临时数据库。HTTP 冒烟拒绝复用
18099 端口上已有的服务，等待本轮进程就绪后再检查 `smoke-routes.txt`。

`WP-M0-03` 的工具链安装、npm ci、版本/摘要一致性、PostgreSQL 扩展构建上下文及独立扫描入口见
[`supply-chain/README.md`](../supply-chain/README.md)。开发期相关检查为 `supply-chain.py check`
与 `python3 scripts/test_supply_chain.py`；不自动加入每次 dev-check，完整扫描留 E01。

`WP-MVP-A01` 的独立 `assets.py build/test/clean` 入口共用 M0-03 的安装器与根 npm 锁文件。
`asset_freeze.py` 对原 A00 冻结做完整字节复核，仅允许已接线的资源 URL 替换；
`test-assets-http.py` 以真实 dist 和临时 loopback 服务检查四环境、资源与 SSR。
操作及失败边界见 [`docs/development/assets.md`](../docs/development/assets.md)。
上述资产构建和针对性验证按变更范围显式运行，不加入每次七步 dev-check。
