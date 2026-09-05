# 搜索图标的独立来源与精确改编

`ui-search.svg` 从 Feather 官方固定 commit `3dc050d97405062eba78aa57115c0a15c63abdaa` 的 [`icons/search.svg`](https://github.com/feathericons/feather/blob/3dc050d97405062eba78aa57115c0a15c63abdaa/icons/search.svg) 改编。MIT 原文与 Cole Bemis 版权声明保留于 [`feather-MIT.txt`](feather-MIT.txt)；原始 SVG 字节以 UTF-8 字符串保存在 [`feather-upstream/search.svg.json`](feather-upstream/search.svg.json)，来源及分发 SHA-256 见 [`feather-search-source.json`](feather-search-source.json)。原始文件没有末尾换行，JSON 容器保存该事实，容器本身遵守仓库 LF 和末尾换行规则。

原图是圆和直线两个独立组件。本轮保留其组件及圆端点属性，分别执行明确的等比缩放和平移：圆的每个坐标乘 `9/16` 再减 `3/16`，半径乘 `9/16`；直线的每个坐标乘 `70/87` 再减 `113/29`。圆因此得到圆心 `(6,6)`、半径 `4.5`；手柄端点得到 `(13,13)` 和 `(9.5,9.5)`。输出视口为 `14×14`，描边宽度改为 `1.4`。这是分别作用于两个组件的变换，不是整图使用同一矩阵。

[`feather-search-geometry.json`](feather-search-geometry.json) 固定此配方与本轮匿名参考的目标几何。资产校验器检查官方原始哈希，从官方圆和直线坐标以有理数重新计算结果，比较完整输出 SVG，并逐项比较参考圆心、半径、端点和描边；只允许直线方向反转，不使用近似容差。

该资产的分发依据是 MIT 授权的 Feather 原作及本项目记录的改编。参考站原 SVG 的来源许可仍未确认，不因几何相等而被改标为 MIT。本证据不授予六视口像素、WCAG 或 `WP-MVP-E04` 视觉门禁通过状态。
