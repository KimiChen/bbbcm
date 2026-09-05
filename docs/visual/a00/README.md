# WP-MVP-A00 本轮开发交付

当前工作包状态与缺口以 `plan/99-traceability.md` 第 5.1 节为准。本目录是交付及证据索引，不是另一套计划事实源。

2026-09-05 从当前公开参考重新采集，未读取 DEC-041 实验产出或仓库外归档。页头品牌使用“草莓论坛”，四个首发版块使用已批准名称。实现遵守 DEC-043 的当前实测值优先；原始截图按 DEC-044 由用户后续处理，旧脱敏参考截图已移除。

## 可复核入口

- `fixtures.json`：14 个本地 SSR 页型、三主题、固定数据与环境边界。
- `reference/`：不含用户文本的结构、计算样式及本地原始截图索引；截图本体位于 Git 忽略的 `docs/evidence/local/a00-redo/reference/`，由用户负责后处理。
- `implementation/`：本项目合成数据的开发预览与计算值，不是正式基准。
- `assets.json`：30 项源资产，完整登记实际许可状态。`licenses/` 保存 OFL/MIT 原文、来源及 DEC-045 本机版本化的未知许可图形记录。
- `structure-freeze.json`：本轮已实现模板标签序列、字面 CSS 类名及源文件哈希的开发冻结清单。后续页面继承这些结构与类名；该清单不是 `visual-baseline-v1`。有意结构变更须先复核并刷新冻结记录。
- `old-names.json`、`checks.json`、`asset-text-scan.json`、`ocr-summary.json`：旧称词表、源码与42种SSR输出扫描、字体/SVG/PNG文本元数据扫描、实现图 OCR 及人工复核结果。
- `profile-observation.json`：匿名公开个人主页的1440/390实测、两张本机原图入口与固定合成资料；`supplemental-forum-menu.json`：全宽版块菜单、站点入口和页脚补测。
- `interactions.md`：开发交互检查和已知移动抽屉背景焦点缺口；不代表完整键盘/WCAG通过。
- `search-control-observation.json`：当前搜索按钮实测与真实鼠标悬停、列表图标13×13外盒；许可证据区分官方上游改编与参考原物。
- `validation.md`：开发期命令、结果和未执行项目。

## 本机复现

1. 以 development/test 配置启动 `BBBCM_CONFIG=<本地配置路径> cargo run --locked -p bbbcm-web`，绑定 `127.0.0.1:18088`；数据库字段仅为现有配置格式，本夹具不使用数据库。
2. 打开 `/__fixtures/a00/home?theme=light`。其余页型和 30 个允许资产 URL 见 `scripts/smoke-routes.txt`。`/` 保留工程骨架页，staging/production 不挂载任何夹具或源资产。
3. `python3 scripts/a00-assets.py --write` 在有意变更源资产后刷新 A00 源清单；平时只运行 `python3 scripts/a00-assets.py`。
4. `python3 scripts/a00-check.py` 检查冻结清单和运行中的本地 SSR；仅经审查的结构更新使用 `--freeze`。
5. `node scripts/a00-preview.mjs` 通过本机 Chrome 9222 采集7个实现预览、计算值及真实资源请求。脚本只访问指定本机夹具，不读取浏览器已有 Cookie/会话。
6. `node scripts/a00-brand-render.mjs --previews` 与 `node scripts/a00-og-render.mjs` 重建本地化品牌资产及本机预览；系统字体和 Chrome 版本需与记录一致。
7. `./scripts/dev-check.sh` 执行开发期最小集；参考采集命令与原始截图边界见 `capture.md`。

Node 使用本机 v26.5.0 原生 WebSocket/fetch（无 npm 依赖），仅用于开发观察；未替代 DEC-032 的 A01 Node/esbuild 构建工具链锁定。A00 静态资源由编译期允许列表提供且 `no-store`；清单中的哈希文件名是 A01 将来构建的输出名，本轮没有创建 dist 或生产源路径。

## 当前边界

本轮未进入 E04：没有执行完整六视口×双主题像素差异、WCAG、正式几何门槛或批准基准。当前计算值与可浏览模板不能代替这些门禁。

本次已补齐品牌章、16/32 favicon、180 应用图标、三帧 ICO、1200×630 OG、共享邮件页头及列表/OAuth 图标。品牌生成记录见 `brand-localization.json`、`og-localization.json`；模板 head 与 CSS 只引用本地允许资源。邮件页头是共享品牌组合，参考邮件未公开观察；成功搜索未公开观察，search 仍复现公开 HTTP-200/显示 200 的错误布局，不臆造私有页面。

未知素材许可按 `DEC-045` 如实标记 `unconfirmed`、`pending` 和 `not-approved`，不阻塞本轮已批准的本机版本化，也不代表第三方授权。对外发布前的权利事项仍由负责人处理。移动抽屉的背景焦点、完整键盘路径和 SVG 动画相位列入 E04。

所有页面数据都是明确固定的合成值；身份认证、搜索、分页等链接只切换夹具，不代表业务能力。此包无数据库、Schema、迁移、业务授权或 Outbox，不建立虚构实现。
