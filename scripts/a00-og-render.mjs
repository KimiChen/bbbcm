// Rebuild A00 localized brand compositions from local SVG code; no remote assets.
import { createHash } from 'node:crypto';
import { execFile } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { createServer } from 'node:http';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(fileURLToPath(new URL('..', import.meta.url)));
const hash = data => createHash('sha256').update(data).digest('hex');
const read = path => readFile(resolve(root, path));
const write = (path, data) => writeFile(resolve(root, path), data);
const brandPath = 'apps/web/static/img/brand-symbol.svg';
const brand = await read(brandPath);
const brandHash = 'f5952515e24fcc527c6cbb26bffb0df8b967defdf1cb0d3adb3f1b5684512a65';
if (hash(brand) !== brandHash) throw new Error('Localized brand source changed; review the composition before rebuilding');
const text = brand.toString('utf8');
const polygon = text.match(/<polygon class="seal"[^>]+\/>/)?.[0];
const glyphs = [...text.matchAll(/<text class="glyph"[^>]*>[^<]+<\/text>/g)].map(match => match[0]);
if (!polygon || glyphs.length !== 2 || !glyphs[0].includes('草莓') || !glyphs[1].includes('论坛')) throw new Error('Unexpected brand source structure');
const seal = polygon.replace('class="seal"', 'stroke="#ffffff"') + '\n' + glyphs.map(glyph => glyph.replace('class="glyph"', 'fill="#ffffff" font-family="PingFang SC, system-ui, sans-serif" font-size="36" font-weight="600" text-anchor="middle" dominant-baseline="central"')).join('\n');
const sealAt = (x, y, size) => `<g transform="translate(${x} ${y}) scale(${size / 200})">\n${seal}\n</g>`;
const ogSvg = `<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="630" viewBox="0 0 1200 630" role="img" aria-labelledby="og-title">
<title id="og-title">草莓论坛</title>
<defs>
  <radialGradient id="og-glow" gradientUnits="userSpaceOnUse" cx="600" cy="225" r="410">
    <stop offset="0" stop-color="#dcc057" stop-opacity="0.16"/>
    <stop offset="0.45" stop-color="#dcc057" stop-opacity="0.09"/>
    <stop offset="1" stop-color="#dcc057" stop-opacity="0"/>
  </radialGradient>
  <pattern id="og-grid" width="60" height="60" patternUnits="userSpaceOnUse">
    <path d="M60 0H0V60" fill="none" stroke="#ffffff" stroke-opacity="0.055" stroke-width="1"/>
  </pattern>
</defs>
<rect width="1200" height="630" fill="#0a0a0a"/>
<rect width="1200" height="630" fill="url(#og-glow)"/>
<rect width="1200" height="630" fill="url(#og-grid)"/>
${sealAt(548, 169, 104)}
<text x="600" y="388" fill="#fafaf7" font-family="PingFang SC, system-ui, sans-serif" font-size="80" font-weight="700" text-anchor="middle">草莓论坛</text>
<path d="M474 444H530M670 444H726" fill="none" stroke="#8d8461" stroke-opacity="0.28" stroke-width="1"/>
<text x="600" y="455" fill="#e6c34e" font-family="PingFang SC, system-ui, sans-serif" font-size="28" font-weight="600" letter-spacing="4" text-anchor="middle">草莓论坛</text>
</svg>
`;
const emailSvg = `<svg xmlns="http://www.w3.org/2000/svg" width="640" height="160" viewBox="0 0 640 160" role="img" aria-labelledby="email-title">
<title id="email-title">草莓论坛</title>
<rect width="640" height="160" fill="#0a0a0a"/>
${sealAt(192, 44, 72)}
<text x="288" y="96" fill="#ffffff" font-family="PingFang SC, system-ui, sans-serif" font-size="40" font-weight="600">草莓论坛</text>
</svg>
`;
const ogPath = 'apps/web/static/img/og-image.svg';
const pngPath = 'apps/web/static/img/og-image.png';
const emailPath = 'apps/web/static/img/email-header.svg';
await write(ogPath, ogSvg);
await write(emailPath, emailSvg);

// Serve only these in-memory files on an ephemeral loopback port. No A01 build/runtime dependency.
const assets = new Map([['/og-image.svg', ogSvg], ['/email-header.svg', emailSvg]]);
const server = createServer((request, response) => {
  const body = assets.get(request.url);
  response.writeHead(body ? 200 : 404, { 'Content-Type': body ? 'image/svg+xml; charset=utf-8' : 'text/plain', 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' });
  response.end(body || 'not found');
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
let socket;
let context;
let call;
const requests = new Set();
let renderer;
try {
  const version = await (await fetch('http://127.0.0.1:9222/json/version')).json();
  socket = new WebSocket(version.webSocketDebuggerUrl);
  await new Promise(resolve => socket.addEventListener('open', resolve, { once: true }));
  let sequence = 0;
  const pending = new Map();
  socket.onmessage = event => {
    const message = JSON.parse(event.data);
    if (message.method === 'Network.requestWillBeSent') requests.add(message.params.request.url);
    if (pending.has(message.id)) {
      const [yes, no] = pending.get(message.id);
      pending.delete(message.id);
      message.error ? no(message.error) : yes(message.result);
    }
  };
  call = (method, params = {}, sessionId) => new Promise((yes, no) => {
    pending.set(++sequence, [yes, no]);
    socket.send(JSON.stringify({ id: sequence, method, params, sessionId }));
  });
  context = await call('Target.createBrowserContext');
  const target = await call('Target.createTarget', { url: 'about:blank', browserContextId: context.browserContextId });
  const { sessionId } = await call('Target.attachToTarget', { targetId: target.targetId, flatten: true });
  const send = (method, params = {}) => call(method, params, sessionId);
  const evaluate = async expression => {
    const result = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (result.exceptionDetails) throw new Error('Local SVG render evaluation failed');
    return result.result.value;
  };
  await send('Page.enable');
  await send('Network.enable');
  await send('Emulation.setScriptExecutionDisabled', { value: true });
  await send('Emulation.setLocaleOverride', { locale: 'zh-CN' });
  const capture = async (path, width, height) => {
    await send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
    const url = origin + path;
    await send('Page.navigate', { url });
    let ready = false;
    for (let index = 0; index < 100; index++) {
      if (await evaluate(`document.readyState==='complete' && location.href===${JSON.stringify(url)}`)) { ready = true; break; }
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    if (!ready) throw new Error('Local SVG timed out');
    await evaluate('document.fonts.ready.then(()=>true)');
    const computed = await evaluate(`(() => ({width:document.documentElement.clientWidth,height:document.documentElement.clientHeight,texts:[...document.querySelectorAll('text')].map(e=>({text:e.textContent,fontFamily:getComputedStyle(e).fontFamily,fontSize:getComputedStyle(e).fontSize,fontWeight:getComputedStyle(e).fontWeight,letterSpacing:getComputedStyle(e).letterSpacing,box:(()=>{const r=e.getBBox();return {x:r.x,y:r.y,width:r.width,height:r.height}})()}))}))()`);
    const bytes = Buffer.from((await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false })).data, 'base64');
    const repeat = Buffer.from((await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false })).data, 'base64');
    if (hash(bytes) !== hash(repeat)) throw new Error('Repeated SVG capture was not deterministic');
    if (bytes.readUInt32BE(16) !== width || bytes.readUInt32BE(20) !== height) throw new Error('PNG dimensions differ from viewport');
    return { bytes, computed, repeatedCaptureIdentical: true };
  };
  const og = await capture('/og-image.svg', 1200, 630);
  await write(pngPath, og.bytes);
  const email = await capture('/email-header.svg', 640, 160);
  await mkdir(resolve(root, '.local/a00-redo'), { recursive: true });
  await write('.local/a00-redo/email-header-preview.png', email.bytes);
  const external = [...requests].filter(url => !url.startsWith(origin + '/'));
  if (external.length) throw new Error('Unexpected external resource request during local SVG rendering');
  renderer = { browser: version.Browser, deviceScaleFactor: 1, zoom: 1, locale: 'zh-CN', javascript: false, animation: 'none in generated assets', network: 'loopback only', externalRequests: external, requests: [...requests].map(url => url.slice(origin.length)), og: { ...og, bytes: undefined }, email: { ...email, bytes: undefined } };
} finally {
  if (context && call) await call('Target.disposeBrowserContext', { browserContextId: context.browserContextId });
  socket?.close();
  await new Promise(resolve => server.close(resolve));
}

const generatorPath = 'scripts/a00-og-render.mjs';
const reportPath = 'docs/visual/a00/og-localization.json';
const documentation = 'docs/visual/a00/og-localization.md';
const referenceUrl = 'https://cache.imgproxy.org/?url=https%3A%2F%2Fsb.sb.sb%2Fattachments%2F2tka5bkfcbik%2Fog-image-1.png&w=1200&h=630&fit=cover&a=attention&output=webp';
const paths = [ogPath, pngPath, emailPath];
const logicalNames = ['og-image-svg', 'og-image', 'email-header'];
const outputs = await Promise.all(paths.map(async (path, index) => ({ logical_name: logicalNames[index], path, sha256: hash(await read(path)) })));
const wordlistPath = 'docs/visual/a00/old-names.json';
const wordlistBytes = await read(wordlistPath);
const terms = JSON.parse(wordlistBytes).terms;
const normalize = value => value.normalize('NFKC').toLowerCase();
for (const [path, content] of [[ogPath, ogSvg], [emailPath, emailSvg]]) {
  if (terms.some(term => normalize(path + '\n' + content).includes(normalize(term)))) throw new Error('Old brand term in generated SVG');
  if (/<script\b|\son\w+\s*=|\s(?:href|src)\s*=/i.test(content)) throw new Error('Unexpected active SVG content');
  for (const match of content.matchAll(/url\(([^)]+)\)/g)) if (!match[1].startsWith('#')) throw new Error('External SVG reference');
}
const png = await read(pngPath);
const pngTextChunks = [];
for (let offset = 8; offset + 12 <= png.length;) {
  const size = png.readUInt32BE(offset);
  const type = png.toString('ascii', offset + 4, offset + 8);
  if (['tEXt', 'zTXt', 'iTXt'].includes(type)) pngTextChunks.push(type);
  offset += size + 12;
}
if (pngTextChunks.length) throw new Error('Generated PNG unexpectedly contains text metadata');
const ocr = JSON.parse(await new Promise((yes, no) => execFile('swift', ['scripts/a00-ocr.swift', '--terms', wordlistPath, pngPath], { cwd: root, maxBuffer: 1024 * 1024 }, (error, stdout) => error ? no(error) : yes(stdout))));
if (ocr.reviewHits !== 0 || ocr.images[0].sha256 !== outputs[1].sha256) throw new Error('Generated PNG OCR failed or describes different bytes');
const report = {
  work_package: 'WP-MVP-A00', local_versioning_decision: 'DEC-045',
  purpose: 'Code-native local brand compositions and deterministic development rendering; not E04 pixel/WCAG validation.',
  reference: { source_url: referenceUrl, sha256: '39b020c979e0f88ec0da7ccd3fe33a7147b6c28ee9ea3a70824905e91e04c706', width: 1200, height: 630, ignored_source_recorded: '.local/a00-redo/reference-og-image.webp', source_binary_committed: false, required_to_rerun_generator: false },
  reference_observations: { method: 'Manual image inspection plus read-only pixel measurements of the current reference WebP; inferred design values are not claimed as original source CSS.', background_sample_rgb: [10, 10, 10], grid_period_px: 60, bright_symbol_pixel_bounds: [548, 169, 652, 273], bright_title_pixel_bounds: [441, 323, 754, 395], subtitle_pixel_bounds: [552, 435, 644, 454] },
  design: { canvas: [1200, 630], background: '#0a0a0a', grid: { step: 60, stroke: '#ffffff', opacity: 0.055, width: 1 }, glow: { center: [600, 225], radius: 410, color: '#dcc057', stops: [[0, 0.16], [0.45, 0.09], [1, 0]] }, symbol: { box: [548, 169, 104, 104], color: '#ffffff' }, title: { text: '草莓论坛', anchor: [600, 388], font_size: 80, weight: 700, color: '#fafaf7' }, subtitle: { text: '草莓论坛', anchor: [600, 455], font_size: 28, weight: 600, letter_spacing: 4, color: '#e6c34e' }, decorative_lines: [[474, 444, 530, 444], [670, 444, 726, 444]] },
  localized_brand: { path: brandPath, sha256: brandHash, modification: 'Inline the same polygon and two localized text rows; fix white stroke/fill for these dark compositions. No source glyph paths or external references are restored.' },
  email: { kind: 'Project-authored shared brand composition', canvas: [640, 160], observed_reference_email: false, explanation: 'Uses only the already observed localized seal and approved brand text. No private reference email page or email layout is asserted.' },
  generator: { path: generatorPath, sha256: hash(await read(generatorPath)), command: 'node scripts/a00-og-render.mjs', source_inputs: [brandPath], remote_asset_requests: false },
  verification: { wordlist: wordlistPath, wordlist_sha256: hash(wordlistBytes), svg_text_scan: 'NFKC plus case-insensitive literal wordlist match: zero hits', svg_external_references: 0, png_text_metadata_chunks: pngTextChunks, png_ocr: { command: `swift scripts/a00-ocr.swift --terms ${wordlistPath} ${pngPath}`, engine: ocr.engine, sha256: outputs[1].sha256, review_hits: ocr.reviewHits, recognized_text: ocr.images[0].observations.map(item => item.text), limit: 'OCR is auxiliary; manual review remains necessary and no E04 gate is asserted.' } },
  renderer, outputs, license: 'unconfirmed', redistributable: 'pending', public_release: 'not-approved',
  rights_boundary: 'The compositions contain localized reference brand geometry whose third-party rights remain unconfirmed. DEC-045 permits local versioning; it is not a third-party license. No AGPL permission is asserted for the inherited geometry.',
  captured_utc: new Date().toISOString(),
};
await write(reportPath, JSON.stringify(report, null, 2) + '\n');
const dimensions = [[1200, 630], [1200, 630], [640, 160]];
const metadata = outputs.map((item, index) => ({
  logical_name: logicalNames[index], path: item.path, sha256: item.sha256,
  content_hash_filename: item.sha256 + (index === 1 ? '.png' : '.svg'), mime: index === 1 ? 'image/png' : 'image/svg+xml',
  width: dimensions[index][0], height: dimensions[index][1], theme: 'fixed dark brand artwork', accessible_name: '草莓论坛',
  source: index === 2 ? 'Project shared brand composition using the localized public seal; no private reference email observed' : 'Code-native localization of the currently observed public OG artwork; generated from local SVG code and localized seal',
  source_url: index === 2 ? 'https://sb.sb.sb/assets/index.svg' : referenceUrl,
  license: 'unconfirmed', redistributable: 'pending', local_versioning_decision: 'DEC-045', public_release: 'not-approved',
  evidence: [reportPath, documentation, generatorPath, brandPath, 'plan/06-decisions-and-risks.md'],
  modification: index === 2 ? 'Compose the localized seal and approved project text into a 640x160 project email header; no reference email claim' : 'Rebuild black grid, central gold glow, white localized seal and brand text as SVG; PNG is a local Chromium rendering of that SVG',
}));
await write('docs/visual/a00/licenses/localized-og-assets.json', JSON.stringify(metadata, null, 2) + '\n');
console.log(JSON.stringify({ assets: outputs, repeated_capture_identical: true, external_requests: 0, report: reportPath }));
