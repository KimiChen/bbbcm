// Development observation only; not the E04 baseline or pixel gate.
// Connect only to local Chrome 9222. Never read cookies/storage or existing tabs.
// DEC-044: raw screenshots are the default and remain in the ignored local evidence directory.
import { mkdir, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import os from 'node:os';
const args = process.argv.slice(2);
if (args.length > 1 || (args.length === 1 && !['--raw', '--sanitized'].includes(args[0]))) {
  throw new Error('Usage: node scripts/a00-capture.mjs [--raw|--sanitized]');
}
const sanitized = args[0] === '--sanitized';
const mode = sanitized ? 'sanitized' : 'raw';
const out = sanitized ? 'docs/visual/a00/reference' : 'docs/evidence/local/a00-redo/reference';
if (!sanitized) {
  // Refuse a raw capture if repository ignore rules no longer protect its fixed output path.
  execFileSync('git', ['check-ignore', '-q', '--', `${out}/manifest.json`], { stdio: 'pipe' });
}
await mkdir(out, { recursive: true });
const version = await (await fetch('http://127.0.0.1:9222/json/version')).json();
const socket = new WebSocket(version.webSocketDebuggerUrl);
await new Promise(resolve => socket.addEventListener('open', resolve, { once: true }));
let next = 0;
const requests = new Map();
const events = new Set();
socket.onmessage = ({ data }) => {
  const message = JSON.parse(data);
  if (message.method) {
    for (const handler of events) handler(message);
    return;
  }
  const pending = requests.get(message.id);
  if (!pending) return;
  requests.delete(message.id);
  message.error ? pending.reject(message.error) : pending.resolve(message.result);
};
function call(method, params = {}, sessionId) {
  return new Promise((resolve, reject) => {
    const id = ++next;
    requests.set(id, { resolve, reject });
    socket.send(JSON.stringify({ id, method, params, sessionId }));
  });
}
const context = await call('Target.createBrowserContext');
const publicPages = [
  ['home', '/'], ['new', '/new/'], ['pinned', '/pinned/'], ['featured', '/featured/'],
  ['forum', '/go/general/'], ['login', '/login/'], ['register', '/register/'],
  ['policy', '/pages/rules/'], ['search', '/search/?q=fixture'],
];
const manifest = { purpose: 'A00 development observation; not an approved baseline', mode, capturedAt: new Date().toISOString(), browser: version.Browser, os: `${os.type()} ${os.release()} ${os.arch()}`, scale: 1, zoom: 1, language: 'zh-CN', collector: 'Codex', authorization: sanitized ? 'User task + DEC-037; final baseline approval remains DEC-019' : 'DEC-044: raw local screenshots for user post-processing; no public redistribution approval', captures: [] };
try {
  const { targetId } = await call('Target.createTarget', { url: 'about:blank', browserContextId: context.browserContextId });
  const { sessionId } = await call('Target.attachToTarget', { targetId, flatten: true });
  const send = (method, params) => call(method, params, sessionId);
  await send('Page.enable');
  await send('Network.enable');
  await send('Network.setBlockedURLs', { urls: [
    '*://*.google-analytics.com/*', '*://*.googletagmanager.com/*',
    ...(sanitized ? ['*://cache.imgproxy.org/*', '*://sb.sb.sb/*'] : []),
  ] });
  // Scripts remain blocked in both modes; sanitized mode also blocks all image/media requests.
  // Never inspect intercepted headers, cookies or body data.
  let interceptionError;
  const documents = new Map();
  events.add(message => {
    if (message.sessionId !== sessionId) return;
    if (message.method === 'Fetch.requestPaused') {
      send('Fetch.failRequest', { requestId: message.params.requestId, errorReason: 'BlockedByClient' })
        .catch(error => { interceptionError = error; });
    }
    if (message.method === 'Network.responseReceived' && message.params.type === 'Document') {
      const { loaderId, frameId, response: { status } } = message.params;
      documents.set(loaderId, { frameId, status });
    }
  });
  await send('Fetch.enable', { patterns: [
    { resourceType: 'Script', requestStage: 'Request' },
    ...(sanitized ? [
      { resourceType: 'Image', requestStage: 'Request' },
      { resourceType: 'Media', requestStage: 'Request' },
    ] : []),
  ] });
  await send('Emulation.setScriptExecutionDisabled', { value: true });
  await send('Emulation.setLocaleOverride', { locale: 'zh-CN' });
  async function evaluate(expression) {
    const r = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (r.exceptionDetails) throw new Error('Observation evaluation failed');
    return r.result.value;
  }
  async function navigate(url) {
    const navigation = await send('Page.navigate', { url });
    if (navigation.errorText || !navigation.loaderId) throw new Error('Public navigation did not start a new document');
    let ready = false;
    for (let n = 0; n < 200; n++) {
      if (interceptionError) throw new Error('Resource blocking failed');
      const response = documents.get(navigation.loaderId);
      if (response?.frameId === navigation.frameId && await evaluate('document.readyState === "complete"')) {
        ready = true;
        break;
      }
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    if (!ready) throw new Error('Public document readiness timed out');
    const fontReady = await evaluate('Promise.race([document.fonts.ready.then(() => true), new Promise(resolve => setTimeout(() => resolve(false), 10000))])');
    if (!fontReady || !await evaluate('document.fonts.status === "loaded"')) throw new Error('Public document font readiness timed out');
    return { status: documents.get(navigation.loaderId).status };
  }
  // Observe a public topic's URL in memory only. Never serialize identifiers/text.
  let topicUrl;
  let forgotUrl;
  for (const [page, path] of publicPages) {
    for (const [width, theme] of (page === 'home' ? [[1440, 'light'], [390, 'light'], [768, 'light'], [760, 'light'], [1440, 'dark']] : [[1440, 'light']])) {
      await send('Emulation.setDeviceMetricsOverride', { width, height: 1000, deviceScaleFactor: 1, mobile: false });
      await send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: theme }, { name: 'prefers-reduced-motion', value: 'reduce' }] });
      const response = await navigate(`https://sb.sb${path}`);
      if (page === 'home' && !topicUrl) topicUrl = await evaluate('document.querySelector(".post-title")?.href');
      // Follow the actual public login link, without guessing a password-recovery route.
      if (page === 'login') forgotUrl = await evaluate('document.querySelector("a.auth-forgot, .auth-forgot a")?.href');
      await capture(page, path, width, theme, response.status);
    }
  }
  if (topicUrl && new URL(topicUrl).origin === 'https://sb.sb') {
    const response = await navigate(topicUrl);
    await capture('topic', '/topic/[public-id-redacted]', 1440, 'light', response.status);
  }
  if (forgotUrl && new URL(forgotUrl).origin === 'https://sb.sb') {
    const response = await navigate(forgotUrl);
    await capture('forgot-password', new URL(forgotUrl).pathname, 1440, 'light', response.status);
  }
  async function capture(page, path, width, theme, httpStatus) {
    const observation = await evaluate(`(() => {
      document.documentElement.dataset.theme = ${JSON.stringify(theme)};
      const style=document.createElement('style'); style.textContent='*,*::before,*::after{animation:none!important;transition:none!important;caret-color:transparent!important}'; document.head.append(style);
      const fields=['color','backgroundColor','fontFamily','fontSize','lineHeight','fontWeight','letterSpacing','display','position','width','height','maxWidth','minHeight','gridTemplateColumns','gap','padding','margin','borderTop','borderBottom','borderLeft','borderRight','borderRadius','boxShadow','overflowX'];
      const nodes=[document.body,...document.querySelectorAll('[class]')];
      const seen=new Set();
      const computed=nodes.filter(e=>{const c=e.tagName.toLowerCase()+(e.getAttribute('class')?'.'+e.getAttribute('class').trim().replace(/\\s+/g,'.'):'');if(seen.has(c)||e.tagName.toLowerCase()==='svg')return false;seen.add(c);return true}).map(e=>({selector:e.tagName.toLowerCase()+(e.getAttribute('class')?'.'+e.getAttribute('class').trim().replace(/\\s+/g,'.'):''),rect:e.getBoundingClientRect().toJSON(),style:Object.fromEntries(fields.map(k=>[k,getComputedStyle(e)[k]]))}));
      const root=getComputedStyle(document.documentElement);
      const tokens=Object.fromEntries([...root].filter(k=>k.startsWith('--')).map(k=>[k,root.getPropertyValue(k).trim()]));
      const errorCode=document.querySelector('.error-code')?.textContent.trim();
      const renderedErrorCode=/^[2-5]\\d{2}$/.test(errorCode||'')?Number(errorCode):null;
      const structure=[...document.body.querySelectorAll('*')].filter(e=>!['SCRIPT','STYLE'].includes(e.tagName)).map(e=>({tag:e.tagName.toLowerCase(),classes:e.getAttribute('class')||'',depth:(()=>{let n=0,p=e;while(p.parentElement){n++;p=p.parentElement}return n})()}));
      const masks=[];
      if(${sanitized}){
      const safe=new Set(['烧饼论坛','综合','AI','域名','主机','硬件','推广','交易','优惠','分享','技术','全部版块','LZ.SB','lz.sb','繁體','登录','注册','全部主题','菜单','×','版块列表','我的菜单','工作','投资','公告','全部','最新回复','最新帖子','置顶','精华','投票','悬赏','抽奖','红包','下一页','上一页','P','访客','请登录后发帖','今日热门','站点统计','主题','回复','用户','最新用户','论坛规则','服务条款','隐私政策','联系我们','认证会员','排行榜','RSS 订阅','邮箱','用户名','密码','记住我','忘记密码？','登录账号','注册账号','邀请码','确认密码','搜索','搜索结果','找回密码','邮箱地址']);
      const forced='.post-title,.post-topic-title,.post-content,.post-content-title,.post-author,.post-meta,.post-time,.post-edited,.post-reply-no,.post-user-uid,.post-user-level,.post-user-group,.sidebar-topic-title,.sidebar-topic-count,.stats-grid,.new-users,.user-name,.user-rank,.verify-badge-value,.reply-light-count';
      function mask(rect, element, reason) {
        let left=Math.max(0,rect.left),top=Math.max(0,rect.top),right=Math.min(document.documentElement.clientWidth,rect.right),bottom=Math.min(innerHeight,rect.bottom);
        // Range rectangles include text hidden by ellipsis/overflow. Respect each original
        // ancestor's padding box so overlays cannot redraw outside its clipped content.
        for(let e=element;e;e=e.parentElement){
          const css=getComputedStyle(e),box=e.getBoundingClientRect();
          if(/hidden|clip|auto|scroll/.test(css.overflowX)){left=Math.max(left,box.left+e.clientLeft);right=Math.min(right,box.left+e.clientLeft+e.clientWidth)}
          if(/hidden|clip|auto|scroll/.test(css.overflowY)){top=Math.max(top,box.top+e.clientTop);bottom=Math.min(bottom,box.top+e.clientTop+e.clientHeight)}
        }
        if(right>left&&bottom>top)masks.push({x:left,y:top,width:right-left,height:bottom-top,reason});
      }
      const walker=document.createTreeWalker(document.body,NodeFilter.SHOW_TEXT);let text;
      while(text=walker.nextNode()){
        if(['SCRIPT','STYLE','OPTION'].includes(text.parentElement.tagName))continue;
        const value=text.textContent.trim();const force=!!text.parentElement.closest(forced);if(!value||(!force&&safe.has(value)))continue;
        const range=document.createRange();range.selectNodeContents(text);
        for(const r of range.getClientRects())if(r.width&&r.height)mask(r,text.parentElement,force?'user content/count container: unconditional mask':'non-allowlisted text or noscript copy');
      }
      for(const e of document.querySelectorAll('img,svg image,video,canvas,iframe,object,embed,.post-avatar,.user-avatar-big,.nu-avatar-circle,.link-card-avatar')){const r=e.getBoundingClientRect();if(r.width&&r.height)mask(r,e,'avatar or unreviewed embedded content');e.style.visibility='hidden';}
      for(const e of document.body.querySelectorAll('*')){if(getComputedStyle(e).backgroundImage!=='none'){const r=e.getBoundingClientRect();if(r.width&&r.height)mask(r,e,'unreviewed CSS image');e.style.backgroundImage='none';}}
      for(const e of document.querySelectorAll('input,textarea')){if(!['submit','button','checkbox','radio'].includes(e.type)){e.value='';e.removeAttribute('value');if(e.placeholder!=='搜索关键词')e.removeAttribute('placeholder');}}
      for(const r of masks){const el=document.createElement('div');el.setAttribute('data-a00-mask','');Object.assign(el.style,{position:'absolute',left:(r.x+scrollX)+'px',top:(r.y+scrollY)+'px',width:r.width+'px',height:r.height+'px',background:'#b8bcc6',zIndex:'2147483647',pointerEvents:'none'});document.body.append(el)}
      }
      return {tokens,computed,structure,masks,bodyWidth:document.documentElement.clientWidth,scroll:{x:scrollX,y:scrollY},fontStatus:document.fonts.status,renderedErrorCode,finalPath:${page === 'topic' ? "'/topic/[public-id-redacted]'" : 'location.pathname'}};
    })()`);
    const stem = `${page}-${width}-${theme}`;
    // Raw output can only reach the fixed ignored directory; sanitized output already has masks.
    const shot = await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
    const bytes = Buffer.from(shot.data, 'base64');
    await writeFile(`${out}/${stem}.png`, bytes);
    await writeFile(`${out}/${stem}.json`, JSON.stringify(observation, null, 2) + '\n');
    manifest.captures.push({ page, url: `https://sb.sb${path}`, finalPath: observation.finalPath, httpStatus, renderedErrorCode: observation.renderedErrorCode, width, height: 1000, theme, screenshot: `${stem}.png`, sha256: createHash('sha256').update(bytes).digest('hex'), observation: `${stem}.json`, masks: observation.masks.length });
    console.log(`${stem}: HTTP ${httpStatus}; ${mode}; ${observation.masks.length} masks`);
  }
  await writeFile(`${out}/manifest.json`, JSON.stringify(manifest, null, 2) + '\n');
} finally {
  await call('Target.disposeBrowserContext', { browserContextId: context.browserContextId });
  socket.close();
}
