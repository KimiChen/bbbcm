# `visual-baseline-v1` 实现基准

`05` §10 的像素差异一律对本基准做回归（几何差异另对参考站截图比对）。
`DEC-057` 冻结基准范围，`DEC-069` 第三项裁定存放方式。

## 存放方式

- **PNG 不入库**。固定写入 `docs/evidence/local/visual-baseline-v1/`，该路径由
  `.gitignore` 第 59 行的 `/docs/evidence/local/` 保护，采集脚本启动前会校验该
  规则仍然有效，失效则拒绝采集。
- **仓库只提交 [`manifest.json`](manifest.json)**，含每张图的 SHA-256、尺寸、
  字节数，以及 `05` §3 要求的完整环境记录。

代价是本基准无法直接用于 CI 像素回归——CI 机器上没有 PNG。这是 `DEC-069`
已知并接受的取舍。

## 复现

```bash
# 需要本机 Chrome 开在 9222，且 A00 夹具服务在 127.0.0.1:18099
node scripts/a00-baseline-capture.mjs
```

比对时用 `manifest.json` 里的 SHA-256 核对本地 PNG 是否与登记一致。

## 范围

10 个 A 级页面 × 6 视口 × 2 主题 = **120 张**。

`05` §6 的 A 级共九类页面，其中**设置页、举报入口、授权 `/manage` 首页**尚无
A00 夹具，不在本次范围内，`manifest.json` 的 `outOfScope` 已登记。

## 两条必须知道的限制

1. **只在 macOS 上可复现**。正文字体是 `system-ui` 栈，实际字形由操作系统提供
   （本次为 SF Pro）。换到 Linux 或 Windows，同一份 CSS 会用另一套字形，文字盒
   宽度随之改变，像素比对必然失败。跨操作系统必须重采。
2. **布局宽度做了归一化**。本机 Chrome 在模拟视口下用覆盖式滚动条（布局宽度=视口），
   而参考侧采集时是经典滚动条（视口−15）。采集时注入
   `html{padding-right:15px;box-sizing:border-box}` 使布局宽度对齐，媒体查询仍按
   标称视口求值。直接把模拟宽度调小 15px 是错的——那会把页面推进下一个断点分支。
