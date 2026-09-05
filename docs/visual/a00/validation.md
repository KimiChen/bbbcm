# A00 开发期验证记录

执行日期：2026-09-05（Asia/Shanghai）；执行者：Codex。当前工作包状态和责任统一见 `plan/99-traceability.md` 第 5.1 节。本记录对应本次实现提交，开发期最高为 `Implemented`，不授予 `Verified` 或发布门禁。

## 已执行

| 命令或检查 | 实际结果与证据 |
|---|---|
| `./scripts/dev-check.sh` | 最终退出 0；格式化、workspace Clippy `-D warnings`、65 个单元测试通过，其中 web 为 8 个。日志见 `dev-check.log.txt`，仅将本机仓库绝对路径替换为 `<repo>`。 |
| Askama 编译期模板检查 | 上述 Clippy/test 编译全部模板；Cargo.lock 锁定 Askama 0.16.1。派生 `Template`、固定 include/extends、`PageFrame`/`ListView` 合同与统一 `Result<Html<String>, ApiError>` 渲染；HTML 转义有针对性测试。 |
| 环境与路由 | 14 页型及 30 项资产逐个验证：development/test 可达，staging/production 为 404；非法主题 400、未知资源 404、账号页 POST 405、查询文本不反射。`/` 保留骨架页，未新增或开启业务 Feature Flag。 |
| SQLite/HTTP 配置冒烟 | `smoke-routes.txt` 的 48 个入口全部符合预期。SQLite 迁移执行器尚未接入，脚本明确跳过，不记为迁移通过。本夹具无数据库、Schema、迁移、业务授权或 Outbox 操作。 |
| `python3 scripts/a00-check.py` | 36 个源码文本文件通过旧称、敏感值与无热链扫描；14×3 共 42 种 SSR 各请求两次，字节一致。30 项实际响应的 SHA-256/MIME/no-store/nosniff 与当前源码清单一致，防止使用旧编译产物。结果见 `checks.json`；结构/CSS 标签序列、类名和源码哈希经复核后 `--freeze`，再以默认模式确认无漂移。 |
| `python3 scripts/a00-assets.py` | 30 项静态源资产与清单双向完整；逻辑名、SHA-256、MIME、尺寸或不适用原因、主题、可访问名称、来源和实际许可状态完整。16 项保留 AGPL/OFL/MIT 依据，14 项如实为 unconfirmed/pending，按 DEC-045 本机版本化，不冒称第三方许可。27 项负向检查见 `assets-validation.json`；先前15项基础与8项菜单检查保留于 `c4dd9b3`、`bdf87f0`，未重复宣称本轮运行。 |
| `.local/a00-redo/asset-scan-venv/bin/python scripts/a00-asset-text-scan.py` | 37 个文件名、4 个 WOFF2 的32条解码名称/可选元数据、17 个 SVG 文本/属性、4 张源 PNG 与7张实现 PNG 的文字/ICC元数据，以及1个 ICO 的3帧容器范围检查全部通过，零旧称命中。见 `asset-text-scan.json`；固定依赖缺失或未知元数据形式会失败，不静默略过。新增解析器针对性验证见 `asset-text-validation.json`。 |
| `node scripts/a00-preview.mjs` | Chrome 152.0.7977.76、缩放/DPR 1、禁用页面 JS；7 个合成预览全部无外部资源请求、无横向溢出，见 `implementation/manifest.json`。字体就绪后等待2秒，SVG动画相位未冻结，不能充当正式像素基准。 |
| 品牌生成与复跑 | `node scripts/a00-brand-render.mjs --previews` 与 `node scripts/a00-og-render.mjs` 只从新编写 SVG 渲染目标图，不编辑参考原 PNG。固定 Chrome、新匿名上下文、零外部资源；同机重复生成字节相同。章形/主题、sRGB图标像素、ICO逐帧与PNG对照、1200×630 OG及共享邮件页头记录见 `brand-localization.json`、`og-localization.json`。系统字体未再分发，跨系统字形度量不作保证。 |
| 栅格旧称 OCR 与人工复核 | `ocr-summary.json` 的命令覆盖15张图：7张合成页面、4张源PNG、明暗章形预览、48px ICO帧及邮件页头预览。全部零命中，人工逐图确认只有合成/目标品牌内容。微小favicon文字依靠源字形和人工补充，OCR不证明任意敏感信息绝对不存在。 |
| 开发交互诊断 | 1440/390搜索按钮悬停/移出与32×26按钮、14×14搜索、13×13列表图标实测见 `search-control-observation.json`。移动菜单打开/关闭/遮罩/Enter、主题GET切换和系统跟随通过；768/1440全宽版块菜单及760隐藏/跨断点见 `supplemental-forum-menu.json`。背景焦点缺口见 `interactions.md`，没有等同 WCAG 通过。 |
| 脚本与 Git | 四份 A00 mjs 的 `node --check`、`bash -n scripts/dev-check.sh`、Python语法/实际执行、`git diff --check` 通过。仅显式路径暂存本轮相关文件；提交前执行 `git diff --cached --check` 和逐文件暂存区复核。 |

旧称词表包括公开参考品牌/域名、原计划目录名及 DEC-025 的历史程序/命名空间。扫描边界见 `old-names.json`。来源和许可证证据保留必要归属名称，不作为运行时旧称。紧凑SVG路径的小数曾被IPv4模式误报；现在仅IPv4检测排除SVG的数值d/points属性，其他文字、属性、旧称及外链扫描保持完整，最终重新检查通过。

## 参考截图与边界

优先连接已有本机 Chrome 9222，创建隔离匿名上下文，未登录或读取 Cookie、已有会话及登录状态。按用户要求移除本轮先前15张脱敏图后，重采15张原始参考PNG至 Git 忽略的 `docs/evidence/local/a00-redo/reference/`；另有2张匿名公开个人主页原图在同级 `profile/`，合计17张。原图哈希与公开索引逐项一致，公开 `reference/` 中 PNG 数为0。原图由用户后处理；公开JSON保留结构/计算值，不保存真实用户名、头像、正文、统计值或个人主页标识。

最终原始搜索截图为 HTTP 200，错误面板显示“200 / 出错了”。此前403记录已按当前实测修正；本批自动提取字段仍为null的原因及人工确认记录保留在参考manifest。成功搜索与私有邮件未公开观察，不臆造页面；邮件页头仅复用本项目本地化品牌。

品牌章、favicon、应用图标、OG和剩余UI/OAuth图形现已本地化。未知许可不再作为本轮本机提交前置（用户更新 `05`、DEC-045），但仍准确标记未确认；不把用户决定当第三方授权。六视口×双主题像素差异、正式几何门槛、完整WCAG（背景焦点、Escape/焦点恢复和SVG动画）及DEC-019基准批准全部留 `WP-MVP-E04`。本轮未执行PostgreSQL/Redis集成、完整E2E、安全、并发或性能验证，也未声称通过。

暂存区复核范围包括Rust/模板/CSS、脚本、计划与文本证据；JSON逐项核对字段与哈希，SVG/字体对照来源许可，PNG/ICO依据上述扫描及人工检查。原始参考PNG、`.local/`、浏览器Profile、会话、密钥、真实部署配置及被忽略的本机AGENTS.md均不暂存。本轮仅本机提交，不部署、不推送。
