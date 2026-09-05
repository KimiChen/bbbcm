// Capture only the explicitly local, synthetic A00 route; not the E04 gate.
import { mkdir, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
const origin = 'http://127.0.0.1:18088';
const out = 'docs/visual/a00/implementation';
await mkdir(out, { recursive: true });
if (!(await fetch(origin + '/health')).ok) throw new Error('Local fixture server is not ready');
const version = await (await fetch('http://127.0.0.1:9222/json/version')).json();
const socket = new WebSocket(version.webSocketDebuggerUrl);
await new Promise(r => socket.addEventListener('open', r, { once: true }));
let id = 0;
const pending = new Map();
const requests = new Set();
socket.onmessage = e => {
  const m = JSON.parse(e.data);
  if (m.method === 'Network.requestWillBeSent') requests.add(m.params.request.url);
  if (pending.has(m.id)) { const [yes, no] = pending.get(m.id); pending.delete(m.id); m.error ? no(m.error) : yes(m.result); }
};
const call = (method, params = {}, sessionId) => new Promise((yes, no) => { pending.set(++id, [yes, no]); socket.send(JSON.stringify({ id, method, params, sessionId })); });
const context = await call('Target.createBrowserContext');
try {
  const { targetId } = await call('Target.createTarget', { url: 'about:blank', browserContextId: context.browserContextId });
  const { sessionId } = await call('Target.attachToTarget', { targetId, flatten: true });
  const send = (method, params) => call(method, params, sessionId);
  await send('Page.enable'); await send('Network.enable');
  await send('Emulation.setScriptExecutionDisabled', { value: true });
  await send('Emulation.setLocaleOverride', { locale: 'zh-CN' });
  const evaluate = async expression => {
    const r = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (r.exceptionDetails) throw new Error('Local preview evaluation failed');
    return r.result.value;
  };
  const captures = [];
  for (const [page, width, theme] of [['home',1440,'light'],['home',390,'light'],['home',1440,'dark'],['login',1440,'light'],['topic',1440,'light'],['profile',1440,'light'],['profile',390,'light']]) {
    await send('Emulation.setDeviceMetricsOverride', { width, height: 1000, deviceScaleFactor: 1, mobile: false });
    const url = `${origin}/__fixtures/a00/${page}?theme=${theme}`;
    await send('Page.navigate', { url });
    let ready = false;
    for (let n = 0; n < 100; n++) { if (await evaluate(`document.readyState === 'complete' && location.href === ${JSON.stringify(url)} && document.documentElement.dataset.fixtureVersion === 'a00-synthetic-v1'`)) { ready = true; break; } await new Promise(r => setTimeout(r, 50)); }
    if (!ready) throw new Error('Local page timed out');
    await evaluate('Promise.race([document.fonts.ready.then(()=>true),new Promise((_,reject)=>setTimeout(()=>reject(new Error("font timeout")),10000))])');
    // Let licensed SVG entrance animations become visible; this is not an E04 animation freeze.
    await new Promise(resolve => setTimeout(resolve, 2000));
    const observation = await evaluate(`(() => {
      const props=['color','backgroundColor','fontFamily','fontSize','lineHeight','fontWeight','letterSpacing','display','padding','gap','borderRadius','gridTemplateColumns'];
      const selectors=['body','.top','.bar','.brand','.wrap','.forum-layout','.post-item','.post-title','.post-meta','.sidebar','.stats-card','.mobile-forum-strip','.auth-shell','.auth-panel','.post-entry','.profile-head','.profile-stats','.profile-toolbar','.a00-profile .user-card','.search-form','.search-btn','.icon-search','.icon-eye','.icon-chat','.icon-user','.icon-forum','.icon-document','.topic-pages','.oauth-btn','.oauth-fixture-icon'];
      const root=getComputedStyle(document.documentElement);
      return {fixtureVersion:document.documentElement.dataset.fixtureVersion,bodyWidth:document.documentElement.clientWidth,scrollWidth:document.documentElement.scrollWidth,tokens:Object.fromEntries([...root].filter(k=>k.startsWith('--')).map(k=>[k,root.getPropertyValue(k).trim()])),computed:selectors.map(selector=>{const el=document.querySelector(selector);return el?{selector,rect:el.getBoundingClientRect().toJSON(),style:Object.fromEntries(props.map(p=>[p,getComputedStyle(el)[p]]))}:{selector,missing:true}})};
    })()`);
    const data = Buffer.from((await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false })).data, 'base64');
    const stem = `${page}-${width}-${theme}`;
    await writeFile(`${out}/${stem}.png`, data);
    await writeFile(`${out}/${stem}.json`, JSON.stringify(observation, null, 2) + '\n');
    captures.push({ page, width, height: 1000, theme, screenshot: `${stem}.png`, sha256: createHash('sha256').update(data).digest('hex'), observation: `${stem}.json`, horizontalOverflow: observation.scrollWidth > observation.bodyWidth });
  }
  const external = [...requests].filter(url => !url.startsWith(origin + '/'));
  const manifest = { purpose: 'A00 developer preview only; not a visual baseline, WCAG or E04 result', browser: version.Browser, scale: 1, zoom: 1, language: 'zh-CN', javascript: false, settleMs: 2000, svgAnimationPhase: 'not frozen; developer preview only', capturedAt: new Date().toISOString(), captures, requests: [...requests].map(url => url.replace(origin,'')), externalRequests: external };
  await writeFile(`${out}/manifest.json`, JSON.stringify(manifest, null, 2) + '\n');
  if (external.length) throw new Error('Unexpected external resource request');
  console.log(JSON.stringify({ captures: captures.length, externalRequests: external.length, overflows: captures.filter(x=>x.horizontalOverflow).map(x=>x.screenshot) }));
} finally { await call('Target.disposeBrowserContext', { browserContextId: context.browserContextId }); socket.close(); }
