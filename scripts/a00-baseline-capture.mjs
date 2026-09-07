// visual-baseline-v1 实现基准采集（`05` §3／§10，`DEC-057`／`DEC-069`）。
//
// 只连本机已开的 Chrome 9222（`AGENTS.md` 要求优先复用该端口），在独立浏览器
// 上下文里开新标签访问 127.0.0.1 的 A00 夹具；不读任何已有标签、cookie 或存储。
//
// PNG 固定写入 Git 忽略的 docs/evidence/local/，仓库只提交本脚本产出的 manifest
// （`DEC-069` 第三项）。脚本启动前校验忽略规则仍然有效，失效则拒绝采集——避免
// 基准图在规则被改动后悄悄进入公开仓库。
import { mkdir, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import os from 'node:os';

const OUT_PNG = 'docs/evidence/local/visual-baseline-v1';
const OUT_MANIFEST = 'docs/visual/a00/baseline/manifest.json';
// `05` §6 A 级：首页四视图、版块页、主题详情、登录/注册、用户主页、静态政策页。
// 设置、举报入口与授权 /manage 首页没有 A00 夹具，本轮不在范围内，manifest 记明。
const PAGES = ['home', 'new', 'pinned', 'featured', 'forum', 'topic', 'login', 'register', 'profile', 'policy'];
const VIEWPORTS = [1440, 1024, 768, 760, 390, 320];
const THEMES = ['light', 'dark'];
const BASE = 'http://127.0.0.1:18099';

execFileSync('git', ['check-ignore', '-q', '--', `${OUT_PNG}/probe.png`], { stdio: 'pipe' });
await mkdir(OUT_PNG, { recursive: true });
await mkdir('docs/visual/a00/baseline', { recursive: true });

const version = await (await fetch('http://127.0.0.1:9222/json/version')).json();
const socket = new WebSocket(version.webSocketDebuggerUrl);
await new Promise(r => socket.addEventListener('open', r, { once: true }));
let next = 0; const reqs = new Map();
socket.onmessage = ({ data }) => {
  const m = JSON.parse(data); if (m.method) return;
  const p = reqs.get(m.id); if (!p) return; reqs.delete(m.id);
  m.error ? p.reject(new Error(JSON.stringify(m.error))) : p.resolve(m.result);
};
const call = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
  const id = ++next; reqs.set(id, { resolve, reject });
  socket.send(JSON.stringify({ id, method, params, sessionId }));
});

const manifest = {
  baseline: 'visual-baseline-v1',
  purpose: '05 §10 像素回归的实现基准；几何比对仍对参考站截图进行',
  authorization: 'DEC-057 冻结基准 + DEC-069 第三项（仓库内只存 manifest）',
  capturedAt: new Date().toISOString(),
  collector: 'Claude Code（本机开发环境）',
  browser: version.Browser,
  os: `${os.type()} ${os.release()} ${os.arch()}`,
  deviceScaleFactor: 1,
  screenshotScale: 1,
  zoom: 1,
  language: 'zh-CN',
  javascript: '页面自身零脚本；采集期仅注入布局宽度归一化样式',
  animations: 'prefers-reduced-motion: reduce',
  scrollPosition: '每张截图前置顶 (0,0)',
  dataStability: 'A00 合成夹具，内容由 fixtures.rs 常量决定，无随机与时间依赖',
  masks: [],
  masksReason: 'A00 夹具全部为合成内容，无用户生成内容需要遮罩；05 §11 列为不可遮罩的品牌、页头、导航与搜索入口本就保留',
  layoutWidthNormalization: 'html{padding-right:15px;box-sizing:border-box}，使布局宽度=视口−15，与参考侧的经典滚动条一致；媒体查询仍按标称视口求值',
  outOfScope: ['设置页', '举报入口', '授权 /manage 首页'],
  outOfScopeReason: '这三类 A 级页面尚无 A00 夹具，无法在冻结夹具集上采集',
  fixturePages: PAGES,
  viewports: VIEWPORTS,
  themes: THEMES,
  fonts: null,
  // 正文字体是 system-ui 栈，实际字形来自操作系统（本次为 macOS 的 SF Pro）。
  // 这意味着本基准**只在同一操作系统 + 同一系统字体**下可复现：换到 Linux 或
  // Windows，同一份 CSS 会用另一套字形，文字盒宽度随之改变，像素比对必然失败。
  // `05` §3 要求记录「字体文件、fallback 和渲染设置」，这条限制是记录的一部分，
  // 不能只登记字体栈就当作可移植。
  reproducibility: '仅在 macOS + 同一系统字体下可复现；跨操作系统需重采基准',
  captures: [],
};

const ctx = await call('Target.createBrowserContext');
try {
  const { targetId } = await call('Target.createTarget', { url: 'about:blank', browserContextId: ctx.browserContextId });
  const { sessionId } = await call('Target.attachToTarget', { targetId, flatten: true });
  const send = (m, p) => call(m, p, sessionId);
  await send('Page.enable');
  await send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });

  for (const page of PAGES) {
    for (const width of VIEWPORTS) {
      for (const theme of THEMES) {
        await send('Emulation.setDeviceMetricsOverride', { width, height: 1000, deviceScaleFactor: 1, mobile: false });
        await send('Page.navigate', { url: `${BASE}/__fixtures/a00/${page}?theme=${theme}` });
        await new Promise(r => setTimeout(r, 1400));
        const { result: probe } = await send('Runtime.evaluate', {
          awaitPromise: true, returnByValue: true,
          expression: `(async () => { try { await document.fonts.ready } catch (e) {}
            let s = document.getElementById('bcm-scrollbar-normalize');
            if (!s) { s = document.createElement('style'); s.id = 'bcm-scrollbar-normalize'; document.head.appendChild(s); }
            // 注意：根元素的 clientWidth 按规范返回视口宽度，不受 html 内边距影响，
            // 因此归一化的判定与回读都必须用 body 的实际盒宽。
            s.textContent = document.body.getBoundingClientRect().width === ${width}
              ? 'html{padding-right:15px;box-sizing:border-box}' : '';
            window.scrollTo(0, 0);
            const faces = [...document.fonts].map(f => f.family + ' ' + f.weight + ' ' + f.status);
            return { layoutWidth: document.body.getBoundingClientRect().width,
              fullHeight: document.documentElement.scrollHeight,
              rendered: !!document.querySelector('.brand') && document.body.innerText.trim().length > 50,
              fontStatus: document.fonts.status, faces,
              bodyFont: getComputedStyle(document.body).fontFamily };
          })()`,
        });
        const info = probe.value;
        if (!info.rendered) throw new Error(`夹具未渲染：${page}@${width}/${theme}`);
        if (info.layoutWidth !== width - 15) throw new Error(`布局宽度未归一：${page}@${width} 得到 ${info.layoutWidth}`);
        manifest.fonts ??= { bodyFontFamily: info.bodyFont, loadedFaces: info.faces };
        await new Promise(r => setTimeout(r, 250));
        const { data } = await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: true,
          clip: { x: 0, y: 0, width: width - 15, height: info.fullHeight, scale: 1 } });
        const bytes = Buffer.from(data, 'base64');
        const name = `${page}-${width}-${theme}.png`;
        await writeFile(`${OUT_PNG}/${name}`, bytes);
        manifest.captures.push({ page, width, theme, file: name,
          layoutWidth: info.layoutWidth, imageHeight: info.fullHeight,
          bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') });
      }
    }
  }
} finally {
  await call('Target.disposeBrowserContext', { browserContextId: ctx.browserContextId });
  socket.close();
}
manifest.count = manifest.captures.length;
await writeFile(OUT_MANIFEST, JSON.stringify(manifest, null, 2) + '\n');
console.log(`采集 ${manifest.count} 张，PNG 在 ${OUT_PNG}/（Git 忽略），manifest 在 ${OUT_MANIFEST}`);
