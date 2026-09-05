# A00 品牌章本地化记录

本记录是本机开发证据，不是视觉基准、WCAG 或 `WP-MVP-E04` 验收。`DEC-045` 允许本轮本机版本化，第三方许可仍为 `unconfirmed`、可再分发性为 `pending`、公开发布为 `not-approved`；不将本项目批准记录当作权利方许可。

本轮公开来源为 `https://sb.sb.sb/assets/index.svg`。源 SHA-256 为 `3ac1426ddaa8208ef0c5bc83ca2432d9eb79f5de87c458283cab016262137f42`，原始观察字节只保存在 Git 忽略的 `.local/a00-redo/reference-index.svg`。可版本化的 `brand-localization.json` 保存源哈希、外框坐标、明暗规则、参考栅格尺寸和背景测量；资产清单与 CI 不要求存在该本地观察文件。

`apps/web/static/img/brand-symbol.svg` 完整保留 200×200 viewBox 内的外框 polygon、13 单位描边，以及浅色 `#0a0a0a` / 深色 `#ffffff` 规则。原有中心字形 path 组全部删除；仅加入“草莓”“论坛”两行文字，使用本机 `PingFang SC` / 系统 fallback，字重 600、字号 36，中心列宽 108。没有复制或捆绑字体文件；资产内没有外部样式、脚本、图片或字体请求。

生成命令为 `node scripts/a00-brand-render.mjs`。脚本从新的本地 SVG 生成每个目标尺寸，使用当前固定 Chrome `152.0.7977.76`、设备缩放 1、无页面脚本的新匿名上下文，随后销毁上下文；页面网络请求必须为零。通过 CDP 诊断执行将该 SVG 直接绘入显式 sRGB canvas，避免本机显示色彩配置把页面截图中的 `#0a0a0a` 转为 `#0d0d0d`。记录的实际平台字体是 `PingFangSC-Semibold`。每个尺寸连续栅格化两次并比较 PNG 字节，另记录每项输出 SHA-256。不同操作系统或字体版本不承诺相同栅格结果。

| 输出 | 尺寸与背景 |
|---|---|
| `brand-symbol.svg` | 固有 512×512；透明底；明暗自适应章形和文字 |
| `favicon-16.png` / `favicon-32.png` | 16×16 / 32×32；深色圆角底、白章；圆角按画布 20%、章形内缩 10% 重建 |
| `apple-touch-icon.png` | 180×180 不透明深色方形底；白章 120×120，外缘位于 30..150 |
| `favicon.ico` | 16/32/48 三帧，32 位 DIB；BGRA 像素加 AND 透明遮罩 |

参考 PNG 的尺寸、透明角与深色底，以及应用图标白章的 120px 外缘均经本轮字节读取与图像复核；没有对参考 PNG 做编辑，也没有将其像素作为生成输入。favicon 的圆角比例与内缩比例由栅格边缘位置重建，并非取得原矢量声明；原参考使用的缩放/抗锯齿算法未确认，本地 Chrome 的边缘像素可能不同。四个中文字符与原字形轮廓必然不同，小尺寸文字可读性与正式像素差异留待 E04。

五项资产的完整 MIME、尺寸、内容哈希、主题、可访问名称及权利状态见 `licenses/localized-brand-assets.json`。源码旧字形删除和两种主题计算值由生成器检查；栅格 OCR 与项目整体旧称扫描由本轮统一校验流程登记。

可加 `--previews` 将 512px 明暗章形预览和 48px ICO 帧 PNG 写入忽略目录 `.local/a00-redo/brand-preview/`，其 `manifest.json` 记录预览哈希。两种 512px 预览分别添加白色与 `#14151c` 背板便于查看透明 SVG；它们不改变正式资产，也不作为公开基准。
