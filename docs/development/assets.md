# A01 静态资产构建与运行

本手册描述已实现的开发命令和接口；计划与状态仍以 [plan/README.md](../../plan/README.md)
和 [99](../../plan/99-traceability.md) 为准。只用于本机开发，正式全量扫描、跨平台构建与视觉矩阵由 E01/E04 执行。

## 构建和本机运行

在仓库根目录，使用 Python 3.11.4+、仓库锁定的 Rust 和 M0-03 安装入口：

```bash
./scripts/supply-chain.py install-tools node
./scripts/supply-chain.py npm-ci
./scripts/assets.py build
cargo run --locked --package bbbcm-web
```

`assets.py` 直接复用 `supply-chain.py` 的工具路径、npm 环境和版本检查，运行根目录的
`assets:build`/`assets:test`/`assets:clean` 脚本。共用 Node 24.20.0、npm 11.19.0、esbuild 0.28.2
与唯一根 `package-lock.json`；不使用全局 Node、不通过 npx 取浮动版本、不另建锁文件。
系统 `python3` 低于 3.11.4 时，用已有的较新 Python 显式运行上述 `.py` 入口，或在当前 shell 的 PATH 中选择它。
缓存已准备时，M0-03 的两个安装命令都可加 `--offline`；构建本身不联网、不更新依赖。

默认本机页面为 [A00 首页](http://127.0.0.1:8080/__fixtures/a00/home)，可切换 `?theme=light|dark|system`。
构建成功后**重启 web**，同一批 A00 SSR 模板的资源 URL 就会从 manifest helper 取得指纹地址。
`/` 仍为冻结的骨架首页，公开品牌外壳和业务读取随 C01 实现；A01 不开放业务 Feature Flag。

| 启动情况 | 行为 |
|---|---|
| 默认 `apps/web/static/dist` 有效 | 四环境均加载并校验 manifest、全部文件及必需资产，建立不可变内存快照 |
| 默认 dist 从未生成，development/test | 日志明确提示 A00 源资产模式；源资源仍仅由固定允许列表提供，使用 no-store |
| dist 已存在但 manifest 缺失/损坏，任意环境 | 启动失败；不得以源路径掩盖构建问题 |
| staging/production 没有 dist | 启动失败，监听 HTTP 前退出 |
| 设置 `BBBCM_ASSET_DIST` | 指定构建目录；所有环境都必须有效，不允许回退；相对路径相对于进程工作目录 |

已启动进程不热读 manifest 或磁盘文件，构建、删除或替换磁盘产物都不会改变它已发出的 immutable URL 对应字节。
采用新产物需重启；后续发布打包只需要二进制与整套 dist，不挂载源目录、manifest 文件、模板文件或目录浏览路由。
A00 编译嵌入的源资源仍只在 development/test 的专用路由可达，环境判定不受 URL 参数或业务开关影响。

## 构建与 manifest 合同

输入为 `apps/web/static/{css,js,img,fonts}/`。当前有 4 份 CSS、22 项图片/图标和 4 份字体；
`js/.gitkeep` 为空占位，不生成运行脚本。JS/TS、模块导入和文件导入能力由合成测试验证。

- 图片/字体保留原始字节，文件名加入完整 SHA-256。
- esbuild 实际编译 CSS/JS/TS。CSS 的 `url()`、`@import` 和 JS 的模块/文件导入经过本地资源解析插件；
  A00 既有绝对源引用仅在构建内映射到受控输入，不改写冻结的 CSS 源文件。
- CSS/JS 输出摘要在依赖 URL 指纹替换之后计算，图片变更会传播到引用它的 CSS/JS 指纹。
- 不压缩、不生成 sourcemap，esbuild 的合法声明保持 inline；许可证原件仍在 A00 的版本化许可目录。
  不更改资产许可，不把未知许可改为已授权。
- 本地导入须写完整扩展名。JS 不隐式导入 CSS，样式入口显式使用 SSR helper；
  远程/裸模块/越界/不存在的引用、目录内符号链接、未知格式和 esbuild 告警均失败。
- manifest 为 UTF-8/LF JSON，格式为 `{"version":1,"assets":[...]}`，条目按逻辑名排序；
  每条含 `logical`、`path`（相对于 dist）、`sha256`、`size` 与 `mime`。没有时钟、主机或绝对路径。
- 范例逻辑名为 `css/site.css`，输出名为 `css/site.<64位小写SHA-256>.css`，URL 为 `/assets/` 加输出名。
  `.ts` 逻辑名对应 `.js` 输出，其余扩展名保持。重复逻辑名/输出路径、未知字段、错误类型/摘要均拒绝。
- 加载上限：manifest 2 MiB、4096 项、单文件16 MiB、内存快照128 MiB。新增类型或提高预算须同时修改构建、加载与测试合同。

构建在临时目录完整完成后更换 dist。编译失败保留旧产物；更换目录时新的启动可能短暂失败，已有内存快照继续服务。
同目录的构建/清理使用 `.dist-lock` 排他锁，竞争直接报错，不自动抢锁。异常中断后先确认没有运行中的构建，
再清理残留的 `apps/web/static/.dist-*` 临时目录；保留其中 `previous` 目录时应先检查并恢复需要的上次产物。
这些目录、dist、node_modules、.local 和 .cache 都由 Git 忽略。

## SSR、路由与品牌覆盖

模板使用 `{{ "css/site.css"|asset_url }}`。`router::render` 通过 Askama 0.16 的 values 注入 `Assets`，
缺少 helper 上下文或未知逻辑名会进入既有 AppError 500 中文通用响应，错误细节只进日志。
禁止拼接生产源 URL 或在 helper 错误时静默输出旧路径。

`GET`/`HEAD /assets/{*path}` 只查 manifest 已校验的内存允许列表，成功响应包含：

```text
Cache-Control: public, max-age=31536000, immutable
X-Content-Type-Options: nosniff
Content-Type: 按扩展名固定并与 manifest 校验的 MIME
```

未知指纹、源名、manifest 或额外磁盘文件返回404；空路径、点段、百分号转义、反斜杠、非法字符或过长路径返回400。
400/404 使用 no-store。查询串不改变资产字节；POST405。服务端不将请求路径拼入磁盘路径。

DEC-035 的 `BrandAssetOverrides` 是未来经审批/审计后的只读品牌覆盖契约，本轮装配为空。
helper 只允许 Logo、favicon 和应用图标逻辑槽位取得覆盖，覆盖优先于构建默认值；CSS、UI 图标、字体、OG 和邮件图不参与。
覆盖包含内容 SHA-256 与非零生效版本，同源对象读取地址约定为 `/media/brand/<SHA-256>.<原槽位扩展名>`，非法覆盖使渲染失败。
该地址只是未来对象读取契约，本轮不挂载媒体路由、不上传、不接对象存储、不实现管理功能。
未来 ADMIN-005 提供该契约时负责格式、尺寸、宽高比、占位盒模型以及审批和审计；像素内容不成为运行时品牌基准。

A00 的 `structure-freeze.json` 保留原文件。`asset_freeze.py` 只将明确的 helper 表达式反向还原为旧 URL，
随后对完整模板字节计算原 SHA-256，其他内容变化都会失败；因此标签顺序、类名、属性、文本和空白均受保护。
`a00-check.py` 共用此规则并只扫描源目录，不把 dist 误收进 A00 源清单。运行检查无需 `--freeze`，不得重置冻结掩盖变化。

## 针对性验证与清理

修改资产管线、manifest helper 或路由时按需运行：

```bash
./scripts/assets.py build
./scripts/assets.py test
python3 scripts/asset_freeze.py
cargo test --locked --package bbbcm-web
python3 scripts/test-assets-http.py --output .cache/a01/http-validation.json
./scripts/dev-check.sh
git diff --check
```

构建测试覆盖真实资产的逐字节重复构建、清单完整性、CSS 4 处字体/11 处图片引用、图片/字体字节保留、
JS/TS 合成模块、依赖指纹传播、失败保留旧产物、符号链接与清理边界。
HTTP 检查使用当前源码、真实 dist 和临时 loopback 服务，覆盖四环境、全部资源 GET/HEAD、SSR helper、错误路径及启动拒绝；
A00 SSR 反向还原资源 URL 后须与既有42种页面/主题响应的字节哈希一致。这是开发期代码/HTTP验证，不代替浏览器视觉证据。
SQLite 配置不表示已执行数据库迁移，迁移执行器仍待 A03；不连接真实 PostgreSQL/Redis。

`dev-check.sh` 仍是原七步最小集，**不无条件执行资产构建或资产全量测试**。
干净检出没有 dist 时使用明确的本地 A00 源模式；指纹成功响应通过上述按需 HTTP 测试验证。

```bash
./scripts/assets.py clean
```

清理只删除可重复生成的 dist，重复执行安全，并拒绝 dist 符号链接；源资产、许可证、冻结记录和锁文件均保留。
要继续验证指纹服务，重新 build 并重启 web。暂存时仅列明源码、配置、测试与脱敏开发证据路径，不提交缓存或产物。
