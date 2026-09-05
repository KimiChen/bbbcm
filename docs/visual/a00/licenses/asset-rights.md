# A00 资产来源与许可调查

采集日期：2026-09-05。执行者：Codex。此文是 `plan/99-traceability.md` 第 5.1 节的资产证据，不是批准记录。仅使用本轮当前公开资源；未使用 `DEC-041` 删除的产出或仓库外归档。

## Space Mono

四个 WOFF2 文件取自 `@fontsource/space-mono@5.3.0` 的 npm 发布包，未修改。官方上游 [googlefonts/spacemono 的 OFL.txt](https://github.com/googlefonts/spacemono/blob/329858c2c4dbd3476f972a4ae00624b018cf4b81/OFL.txt) 明确使用 OFL-1.1；本地分发副本为 [`space-mono-OFL-1.1.txt`](space-mono-OFL-1.1.txt)，归属 Space Mono Project Authors。该副本仅删除上游第 21 行末尾的一个 ASCII 空格以满足仓库空白检查；版权、许可措辞与换行均未改变，因此不声称本地副本与上游原始字节完全相同。字体不改用本项目 AGPL 许可证。

[`space-mono-source.json`](space-mono-source.json) 固定上游 commit、npm 发布包 URL、版本、SHA-512 SRI、SHA-1、SHA-256，并分别登记上游原始许可 SHA-256、本地分发许可 SHA-256 与上述空白规范化操作。资产校验使用本地哈希，另通过恢复第 21 行末尾的空格验证可还原到已确认的上游原始哈希；不会混淆两份字节。[`space-mono-fontsource-metadata.json`](space-mono-fontsource-metadata.json) 记录同一发布包中的 Google Fonts v17 元数据；[`font-assets.json`](font-assets.json) 逐个登记源文件、完整 SHA-256、MIME、字重、子集与证据。发布包 SHA-512 SRI 与 SHA-1 已与 npm 元数据比较并通过。

本轮再次直接请求当前公开参考 CSS 所指的四个带 `?v=` 资源，与上述包中的二进制逐字节比较，全部相同。对应 Latin 400、Latin 700、Latin-ext 400、Latin-ext 700 的 SHA-256 前缀依次为 `fb4a81a2d0`、`2d46bd159b`、`44ad50c760`、`52edd9a895`。来源 URL 只在证据 JSON 中登记；运行时仅使用 `apps/web/static/fonts/` 的本地资源。

随软件分发时保留版权声明和 OFL-1.1 全文，不单独出售字体，不宣称字体作者为本项目背书。原始 OFL 许可未声明 Reserved Font Name；本轮四个字体二进制与字体名称均不变。

字体固有宽高不适用，清单以 `null` 加原因记录，不伪造栅格尺寸。UI 主字体为操作系统字体栈；本轮未复制或再分发操作系统字体。

## 页头文字品牌

[`brand-mark.svg`](../../../../apps/web/static/img/brand-mark.svg) 为本轮用文本和基础矩形重新编写的项目资产，文字仅为“草莓论坛”，按根目录 `LICENSE` 随项目使用 AGPL-3.0-only。只复制已观察的标签设计 token：黄色 `#ffd02f`、文字 `#1c1c1e`、16px / 600、内边距 3px / 10px、圆角 6px；本轮浏览器计算的行高为 28px，因此整体为 84×34。字距按当前公开计算值与 `DEC-043` 使用 `-0.01em`（16px 下为 -0.16px）。具体截图与计算值以本工作包主证据为准。

这份 SVG 不含参考站品牌字形路径，不含第三方字体二进制，不替代参考站 favicon 的图形。SVG 中使用系统字体，最终文本度量取决于固定浏览器和系统字体；六视口、几何和像素门禁仍留待 `WP-MVP-E04`。

## 未确认的品牌图形与未观察资产

当前公开的 [参考 SVG](https://sb.sb.sb/assets/index.svg) 是带边框的字母路径标记，与页头黄色纯文字标签不同。该文件未含许可证；现有公开观察未提供可再分发授权。其原始字节仅保留在 Git 忽略的 `.local/a00-redo/reference-index.svg`，完整 SHA-256 见 [`reference-brand-assets.json`](reference-brand-assets.json)。按用户修改后的 `05` 与 `DEC-045`，本轮允许本地化后在本机仓库版本化；不得改标为本项目自有资产或冒称第三方已经授权。

DOM 中公开的 favicon 32×32、16×16、Apple touch icon 和 favicon.ico URL 已作本轮直接获取。默认 Python User-Agent 首次访问失败，显式使用普通浏览器 User-Agent 后四项均返回 HTTP 200；图像文件解析通过，尺寸分别为 32×32、16×16、180×180，以及包含 16×16、32×32、48×48 三帧的 ICO。Apple touch icon 与 PNG 人工查看为相同字母品牌章，不含用户信息；元数据未提供可再分发许可。原文件同样仅留在 `.local/a00-redo/`，未进入公开静态目录。已观察 URL、哈希、尺寸和状态见 [`reference-brand-assets.json`](reference-brand-assets.json)。

后续对本轮公开首页 `<head>` 的定向检查发现 `og:image` 与 `twitter:image` 指向同一张 1200×630 WebP；已匿名获取并人工确认是黑色网格背景、白色参考品牌章和文字的品牌图，没有用户内容。原图只保留在 `.local/a00-redo/reference-og-image.webp`，URL、SHA-256 与尺寸见同一清单；可再分发许可仍待确认。参考邮件品牌图未公开观察，不臆造邮件布局。目标品牌 SVG、favicon、应用图标与 OG 已按当前公开章形/色彩/布局本地化；邮件页头只组合相同的目标品牌资产，明确属于本项目复用。具体生成方式与文字替换见 `../brand-localization.json`、`../og-localization.json`。本机版本化按 `DEC-045`，未知第三方权利仍记录为 `unconfirmed`/`pending`，不是对外发布许可；开发交付与 E04 正式门禁分别登记。

隔离清单也显式列出每项逻辑名、SHA-256、MIME、尺寸、主题、可访问名称、来源和权利字段。尚未观察或未验证的值使用 `null` 加原因；可访问名称是本项目的清单标签，不冒充参考站的 ARIA 属性。六份原物有可复核的本机路径与哈希，邮件图未获取；这些来源证据与 `license_evidence` 分开，后者尚为空。`content_hash_filename` 为空表示没有可分发的指纹输出，不能把隔离原物当成已经交付的目标品牌资产。

## 已确认的主题 UI 图标

太阳与月亮图标已匹配到 [Material Line Icons 官方项目](https://github.com/cyberalien/line-md) 的 `moon-filled-to-sunny-filled-loop-transition` 与 `moon-filled-loop`。固定上游 commit 为 `2ed22555cee9c1e50d4269865681d01ee8cffd7c`，许可证为 MIT，版权归 Vjacheslav Trushkin。原文保留在 [`line-md-MIT.txt`](line-md-MIT.txt)，固定版本 URL 与原始 SHA-256 保留在 [`line-md-source.json`](line-md-source.json)。

`apps/web/static/img/theme-sun.svg` 与 `theme-moon.svg` 是未修改的官方源文件，只改本地文件名。SVG 使用 `currentColor` 和内置声明式 SVG 动画，没有脚本或外部请求；供模板继承主题与可访问名称。许可说明与上述版权必须随分发保留。

本轮只从匿名公开页面提取这两个 SVG 的标签、几何与动画属性，不保存周边文本。[`ui-icon-comparison.json`](ui-icon-comparison.json) 登记比较方法和结果；`reference-ui/` 保存已确认许可的参考 SVG 证据。将相对/绝对路径与简写曲线归一化后，太阳 4 条、月亮 5 条有效形状路径全部一致，完整节点树、几何、描边/填充与动画属性也一致。仅忽略根尺寸/展示类名/`aria-hidden`、本地遮罩 ID、透明画布路径以及 `.4s`/`0.4s` 等等价语法；不跳过可见图形或动画差异。这是来源匹配证据，不是像素或 WCAG 门禁。

菜单、搜索、用户、浏览、评论等参考图标与 Lucide/Tabler 对应官方路径并不一致，不能把库的许可证套用到相似图形。具体未匹配项与原因也登记在上述比较 JSON；OAuth Google/GitHub/X 品牌图形的本轮来源另见下节，不套用这些上游库的许可证。Lucide 当前主许可为 ISC，继承的部分 Feather 图标另带 MIT；本轮未引入或宣称这些未匹配图形已获许可。

## 从已许可上游改编的基础 UI 图标

后续公开几何复核补齐了菜单与向上箭头两项：它们以 Feather 固定 commit `3dc050d97405062eba78aa57115c0a15c63abdaa` 的官方 MIT 源为起点，保留版权与许可全文，按记录的仿射变换生成本地 SVG。全部坐标乘 `2/3`；菜单再在两轴平移 `1.5`，视口分别为 19×19 和 16×16；描边调整为本轮观察的 `1.7`，圆端点和圆连接保留。

[`feather-source.json`](feather-source.json)、[`feather-icons-assets.json`](feather-icons-assets.json) 和 [`feather-geometry.json`](feather-geometry.json) 记录官方版本、上游原始字节、修改方式、分发哈希与目标几何。校验器从留存的官方坐标重新计算变换结果，并对本地 SVG 的节点与属性逐项比较；不是仅接受一个“相同”的布尔值。菜单在本地移动入口使用；向上箭头作为本轮 UI 图标集交付，目前没有接入新的业务控件。

这两项的分发依据是已许可的 Feather 源与本项目明确记录的改编，不声称参考站未知来源的原 SVG 已获 MIT 授权。前节“未匹配到参考原图来源”的调查结论仍成立，但这两项已不再缺精确的可分发替代实现。用户、文档/主题、版块及 OAuth 图形本轮改按 `DEC-045` 登记原始公开几何和未知许可，见下节。

搜索、浏览与评论现已由同一固定 Feather MIT 上游的明确改编补齐：搜索保留圆与直线，分别按有理数缩放/平移；浏览保留四段平滑三次贝塞尔曲线及瞳孔，调整三个对称参数；评论保留原路径命令结构，调整圆角、下边与尾部参数。原始字节、许可、修改方式、分发哈希及独立重建比较分别见 [`feather-search-provenance.md`](feather-search-provenance.md) 和 [`feather-shape-provenance.md`](feather-shape-provenance.md)。搜索盒14×14、按钮32×26、浏览/评论展示盒13×13及真实悬停颜色来自 `../search-control-observation.json`；不将内部24×24视口误当展示尺寸。

## 按 DEC-045 本机版本化的图形

`reference-ui-pending-assets.json` 的用户、主题分页、版块与 Google/GitHub/X 六项来源于本轮匿名公开页面；`reference-ui-pending-source.json` 保留选定 SVG 树或静态响应字节、来源与分发哈希。图形和属性保留，独立 SVG 只补 namespace、固有尺寸或原继承的 currentColor。Google 保留彩色响应原字节。它们已接入本地 CSS/模板；OAuth 控件仍为无认证行为的展示按钮。

`localized-brand-assets.json`、`localized-og-assets.json` 逐项登记目标品牌章形、favicon、应用图标、OG 和共享邮件页头。目标文字为“草莓论坛”；章形保留本轮公开 SVG 的精确多边形及主题色，栅格背景与 OG 布局按当前图像测量重建，并记录可重跑的 Chrome 渲染器。没有把重绘文字后的像素等同于原参考，也没有授予 E04 门禁。已观察的原始品牌文件继续留在忽略目录，仅用于来源复核；邮件原物未观察的事实保持不变。

全部未知许可资产仍标 `license=unconfirmed`、`redistributable=pending`、`local_versioning_decision=DEC-045`、`public_release=not-approved`。这是用户对本轮本机版本化前置的修改，不是第三方许可证明。已确认 OFL/MIT 的素材仍携带原版权与许可全文；本轮不部署、不推送。
