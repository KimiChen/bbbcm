# 浏览与评论图标的路径参数改编

两项资产都以 Feather 官方固定 commit `3dc050d97405062eba78aa57115c0a15c63abdaa` 的 MIT 源开始；版权仍归 Cole Bemis，许可全文见 [`feather-MIT.txt`](feather-MIT.txt)。源文件的准确 URL、原始 SHA-256 与分发 SHA-256 见 [`feather-shape-source.json`](feather-shape-source.json)。`feather-upstream/eye.svg.json` 和 `feather-upstream/message-square.svg.json` 内保存原始 UTF-8 字节字符串，包括上游没有末尾换行的事实。

`ui-eye.svg` 保留 [`eye.svg`](https://github.com/feathericons/feather/blob/3dc050d97405062eba78aa57115c0a15c63abdaa/icons/eye.svg) 的四段平滑三次贝塞尔曲线与中央圆。以 `(12,12)` 为中心，把半宽 `11` 改为 `9.5`，半高 `8` 改为 `6`，外端控制偏移 `4` 改为 `3.5`。瞳孔半径仍为 `3`。这是原路径的三参数改编，不是整图仿射缩放。

`ui-chat.svg` 保留 [`message-square.svg`](https://github.com/feathericons/feather/blob/3dc050d97405062eba78aa57115c0a15c63abdaa/icons/message-square.svg) 的移动、三段圆弧、水平/垂直/斜直线及闭合命令结构。左、右、上边保持 `3`、`21`、`3`，圆角半径从 `2` 改为 `4`，下边从 `17` 改为 `19`，尾部上端 x 从 `7` 改为 `8`，尾尖 y 从 `21` 改为 `22`。没有添加或替换未知来源的路径片段。

两图保持 `24×24` 内部视口、描边 `2`、圆连接和原组件结构；展示尺寸由本项目 CSS 决定。校验器从固定源字节开始，用原始参数重建整条官方路径，再代入 [`feather-shape-geometry.json`](feather-shape-geometry.json) 中的目标参数重建分发 SVG。比较使用精确有理数，展开相对坐标、平滑曲线的反射控制点、水平/垂直线和圆弧标记，无舍入容差；目标与本轮匿名公开观察逐坐标相等。只调整参数，不将参考原物套用 Feather 许可证。

分发依据是已许可官方源和明确记录的本项目改编；参考站原图许可仍未知。这个构造检查不等于像素、WCAG 或 `WP-MVP-E04` 门禁通过。用户头像的身体轮廓包含不同的路径结构，本轮没有据此交付未经论证的替代图形。
