// 用本机已开的 Chrome 9222（与参考观察同版本）量本站夹具的页头几何。
// 只在独立浏览器上下文里开新标签访问 127.0.0.1 夹具，不读任何已有标签、cookie 或存储。
import { readFileSync, writeFileSync } from 'node:fs';
import os from 'node:os';
const version = await (await fetch('http://127.0.0.1:9222/json/version')).json();
const socket = new WebSocket(version.webSocketDebuggerUrl);
await new Promise(r => socket.addEventListener('open', r, { once: true }));
let next = 0; const requests = new Map();
socket.onmessage = ({ data }) => {
  const m = JSON.parse(data); if (m.method) return;
  const p = requests.get(m.id); if (!p) return; requests.delete(m.id);
  m.error ? p.reject(new Error(JSON.stringify(m.error))) : p.resolve(m.result);
};
const call = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
  const id = ++next; requests.set(id, { resolve, reject });
  socket.send(JSON.stringify({ id, method, params, sessionId }));
});
const SELECTORS = JSON.parse(readFileSync(process.argv[2], 'utf8'));
const VIEWPORTS = [1440, 1024, 768, 760, 390, 320];
const context = await call('Target.createBrowserContext');
const out = { browser: version.Browser, os: `${os.type()} ${os.release()} ${os.arch()}`,
  capturedAt: new Date().toISOString(), scale: 1, zoom: 1, language: 'zh-CN', viewports: {} };
try {
  const { targetId } = await call('Target.createTarget', { url: 'about:blank', browserContextId: context.browserContextId });
  const { sessionId } = await call('Target.attachToTarget', { targetId, flatten: true });
  const send = (m, p) => call(m, p, sessionId);
  await send('Page.enable'); await send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
  for (const width of VIEWPORTS) {
    await send('Emulation.setDeviceMetricsOverride', { width, height: 1000, deviceScaleFactor: 1, mobile: false });
    await send('Page.navigate', { url: `http://127.0.0.1:18099/__fixtures/a00/home?theme=light` });
    await new Promise(r => setTimeout(r, 2500));
    const { result } = await send('Runtime.evaluate', {
      awaitPromise: true, returnByValue: true,
      expression: `(async () => { try { await document.fonts.ready } catch(e) {}
        const sels = ${JSON.stringify(SELECTORS)}; const o = {};
        for (const s of sels) { const e = document.querySelector(s);
          if (!e) { o[s] = null; continue; }
          const r = e.getBoundingClientRect();
          o[s] = { x:+r.x.toFixed(3), y:+r.y.toFixed(3), width:+r.width.toFixed(3), height:+r.height.toFixed(3) }; }
        o.__meta = { bodyWidth: document.body.getBoundingClientRect().width,
          fontStatus: document.fonts.status, navItems: document.querySelectorAll('.forum-link').length,
          scrollW: document.documentElement.scrollWidth, clientW: document.documentElement.clientWidth };
        return o; })()`,
    });
    out.viewports[width] = result.value;
  }
} finally {
  await call('Target.disposeBrowserContext', { browserContextId: context.browserContextId });
  socket.close();
}
writeFileSync(process.argv[3], JSON.stringify(out, null, 1));
console.log('采集完成', out.browser, '视口', Object.keys(out.viewports).join('/'));
