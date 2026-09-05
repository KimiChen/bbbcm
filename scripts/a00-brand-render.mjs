// Rebuild only the localized SVG's own icons. Never read or edit reference PNGs.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { platform, arch, release } from 'node:os';
import { inflateSync } from 'node:zlib';

const expectedBrowser = 'Chrome/152.0.7977.76';
const assetDir = 'apps/web/static/img';
const sourcePath = `${assetDir}/brand-symbol.svg`;
const evidencePath = 'docs/visual/a00/brand-localization.json';
const metadataPath = 'docs/visual/a00/licenses/localized-brand-assets.json';
const svg = await readFile(sourcePath, 'utf8');
assert.ok(svg.includes('>草莓</text>') && svg.includes('>论坛</text>'));
assert.ok(svg.includes('points="6.5,6.5 193.5,6.5 193.5,76 177.5,100 193.5,124 193.5,193.5 6.5,193.5 6.5,124 22.5,100 6.5,76"'));
assert.ok(!/(?:https?:\/\/|data:|<script|<image|<path)/i.test(svg.replace('http://www.w3.org/2000/svg', '')));
const hash = bytes => createHash('sha256').update(bytes).digest('hex');

// Decode our Chrome canvas PNG only, to store the same pixels as 32-bit ICO DIBs.
function pngRgba(png) {
  assert.equal(png.subarray(0, 8).toString('hex'), '89504e470d0a1a0a');
  const width = png.readUInt32BE(16), height = png.readUInt32BE(20);
  assert.equal(png[24], 8); assert.ok([2, 6].includes(png[25])); assert.equal(png[28], 0);
  const channels = png[25] === 6 ? 4 : 3, stride = width * channels, chunks = [];
  for (let off = 8; off < png.length;) {
    const len = png.readUInt32BE(off), type = png.toString('ascii', off + 4, off + 8);
    if (type === 'IDAT') chunks.push(png.subarray(off + 8, off + 8 + len));
    off += len + 12;
  }
  const raw = inflateSync(Buffer.concat(chunks)), scan = Buffer.alloc(stride * height);
  const paeth = (a, b, c) => { const p = a + b - c, aa = Math.abs(p - a), bb = Math.abs(p - b), cc = Math.abs(p - c); return aa <= bb && aa <= cc ? a : bb <= cc ? b : c; };
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)]; assert.ok(filter <= 4);
    for (let x = 0; x < stride; x++) {
      const a = x >= channels ? scan[y * stride + x - channels] : 0;
      const b = y ? scan[(y - 1) * stride + x] : 0;
      const c = y && x >= channels ? scan[(y - 1) * stride + x - channels] : 0;
      const predictor = [0, a, b, Math.floor((a + b) / 2), paeth(a, b, c)][filter];
      scan[y * stride + x] = (raw[y * (stride + 1) + 1 + x] + predictor) & 255;
    }
  }
  const rgba = Buffer.alloc(width * height * 4);
  for (let p = 0; p < width * height; p++) { scan.copy(rgba, p * 4, p * channels, p * channels + 3); rgba[p * 4 + 3] = channels === 4 ? scan[p * channels + 3] : 255; }
  return { width, height, rgba };
}

function icoDib(png) {
  const { width, height, rgba } = pngRgba(png), maskStride = Math.ceil(width / 32) * 4;
  const dib = Buffer.alloc(40 + width * height * 4 + maskStride * height);
  dib.writeUInt32LE(40); dib.writeInt32LE(width, 4); dib.writeInt32LE(height * 2, 8);
  dib.writeUInt16LE(1, 12); dib.writeUInt16LE(32, 14); dib.writeUInt32LE(width * height * 4, 20);
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    const input = (y * width + x) * 4, row = height - 1 - y, output = 40 + (row * width + x) * 4;
    dib[output] = rgba[input + 2]; dib[output + 1] = rgba[input + 1]; dib[output + 2] = rgba[input]; dib[output + 3] = rgba[input + 3];
    if (rgba[input + 3] === 0) dib[40 + width * height * 4 + row * maskStride + (x >> 3)] |= 128 >> (x & 7);
  }
  return { width, height, dib };
}

function makeIco(pngs) {
  const frames = pngs.map(icoDib), header = Buffer.alloc(6 + frames.length * 16);
  header.writeUInt16LE(1, 2); header.writeUInt16LE(frames.length, 4); let offset = header.length;
  frames.forEach((f, i) => { const p = 6 + i * 16; header[p] = f.width; header[p + 1] = f.height; header.writeUInt16LE(1, p + 4); header.writeUInt16LE(32, p + 6); header.writeUInt32LE(f.dib.length, p + 8); header.writeUInt32LE(offset, p + 12); offset += f.dib.length; });
  return Buffer.concat([header, ...frames.map(f => f.dib)]);
}

const version = await (await fetch('http://127.0.0.1:9222/json/version')).json();
assert.equal(version.Browser, expectedBrowser, 'Use the recorded Chrome version for this deterministic renderer');
const socket = new WebSocket(version.webSocketDebuggerUrl);
await new Promise(resolve => socket.addEventListener('open', resolve, { once: true }));
let id = 0; const pending = new Map(), requests = [];
socket.onmessage = e => { const m = JSON.parse(e.data); if (m.method === 'Network.requestWillBeSent') requests.push(m.params.request.url); if (pending.has(m.id)) { const p = pending.get(m.id); pending.delete(m.id); m.error ? p.reject(m.error) : p.resolve(m.result); } };
const call = (method, params = {}, sessionId) => new Promise((resolve, reject) => { pending.set(++id, { resolve, reject }); socket.send(JSON.stringify({ id, method, params, sessionId })); });
const { browserContextId } = await call('Target.createBrowserContext');
try {
  const { targetId } = await call('Target.createTarget', { url: 'about:blank', browserContextId });
  const { sessionId } = await call('Target.attachToTarget', { targetId, flatten: true });
  const send = (method, params) => call(method, params, sessionId);
  await send('Page.enable'); await send('Network.enable'); await send('DOM.enable'); await send('CSS.enable');
  await send('Emulation.setScriptExecutionDisabled', { value: true });
  await send('Emulation.setLocaleOverride', { locale: 'zh-CN' });
  await send('Emulation.setDefaultBackgroundColorOverride', { color: { r: 0, g: 0, b: 0, a: 0 } });
  const evaluate = async expression => { const r = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true }); if (r.exceptionDetails) throw new Error('Local SVG evaluation failed'); return r.result.value; };
  const fontRecords = [], iconPixels = new Map(), renderRecords = [];
  async function render(size, kind, theme) {
    await send('Emulation.setDeviceMetricsOverride', { width: size, height: size, deviceScaleFactor: 1, mobile: false });
    await send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value: theme }] });
    const inset = kind === 'app' ? size / 6 : kind === 'favicon' ? size / 10 : 0;
    const nested = svg.replace('width="512" height="512"', `x="${inset}" y="${inset}" width="${size - inset * 2}" height="${size - inset * 2}"`);
    const backdrop = kind === 'preview' ? (theme === 'dark' ? '#14151c' : '#ffffff') : '#0a0a0a';
    const background = kind === 'symbol' ? '' : `<rect width="${size}" height="${size}" rx="${kind === 'favicon' ? size / 5 : 0}" fill="${backdrop}"/>`;
    const html = `<!doctype html><html lang="zh-CN"><meta charset="utf-8"><style>html,body{margin:0;padding:0;width:${size}px;height:${size}px;overflow:hidden;background:transparent}body>svg{display:block}</style><body><svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 ${size} ${size}">${background}${nested}</svg></body></html>`;
    const frameId = (await send('Page.getFrameTree')).frameTree.frame.id;
    await send('Page.setDocumentContent', { frameId, html });
    await evaluate('document.fonts.ready.then(()=>true)');
    const appearance = await evaluate(`({seal:getComputedStyle(document.querySelector('.seal')).stroke,glyphs:[...document.querySelectorAll('text')].map(e=>({text:e.textContent,fill:getComputedStyle(e).fill,fontFamily:getComputedStyle(e).fontFamily,fontWeight:getComputedStyle(e).fontWeight,bbox:Object.fromEntries(['x','y','width','height'].map(k=>[k,e.getBBox()[k]]))}))})`);
    const expected = theme === 'dark' ? 'rgb(255, 255, 255)' : 'rgb(10, 10, 10)';
    assert.equal(appearance.seal, expected); assert.ok(appearance.glyphs.every(x => x.fill === expected));
    const { root } = await send('DOM.getDocument'); const { nodeIds } = await send('DOM.querySelectorAll', { nodeId: root.nodeId, selector: 'text' });
    const fonts = await Promise.all(nodeIds.map(nodeId => send('CSS.getPlatformFontsForNode', { nodeId })));
    fontRecords.push(...fonts.flatMap(x => x.fonts));
    // An explicitly sRGB canvas avoids the host display profile changing #0a0a0a
    // into #0d0d0d in Page.captureScreenshot. Input is our SVG, never a source PNG.
    const capture = async () => Buffer.from(await evaluate(`(async () => {
      const markup=new XMLSerializer().serializeToString(document.querySelector('body>svg'));
      const bitmap=new Image();bitmap.src='data:image/svg+xml;charset=utf-8,'+encodeURIComponent(markup);
      await bitmap.decode();const canvas=document.createElement('canvas');canvas.width=${size};canvas.height=${size};
      const ctx=canvas.getContext('2d',{colorSpace:'srgb'});ctx.drawImage(bitmap,0,0);
      return canvas.toDataURL('image/png').split(',')[1];
    })()`), 'base64');
    const png = await capture(); assert.deepEqual(await capture(), png, 'Two consecutive SVG renders must be byte identical');
    const pixels = pngRgba(png); assert.equal(pixels.width, size); assert.equal(pixels.height, size);
    renderRecords.push({ kind, size, theme, appearance, png_sha256: hash(png), deterministic_repeat: true });
    return png;
  }
  await render(512, 'symbol', 'light'); await render(512, 'symbol', 'dark');
  for (const size of [16, 32, 48]) iconPixels.set(size, await render(size, 'favicon', 'dark'));
  const app = await render(180, 'app', 'dark'), ico = makeIco([16, 32, 48].map(size => iconPixels.get(size)));
  const externalRequests = requests.filter(url => !url.startsWith('data:image/svg+xml'));
  assert.deepEqual(externalRequests, [], 'The brand renderer must make no page network requests');
  const uniqueFonts = [...new Map(fontRecords.map(f => [JSON.stringify(f), f])).values()];
  assert.ok(uniqueFonts.length && uniqueFonts.every(f => f.familyName === 'PingFang SC' && !f.isCustomFont), 'Recorded system PingFang SC font is required');
  const outputs = [
    ['brand-symbol', 'brand-symbol.svg', Buffer.from(svg), 'image/svg+xml', 512, 512, 'adaptive-light-dark'],
    ['favicon-16', 'favicon-16.png', iconPixels.get(16), 'image/png', 16, 16, 'fixed-dark-background'],
    ['favicon-32', 'favicon-32.png', iconPixels.get(32), 'image/png', 32, 32, 'fixed-dark-background'],
    ['app-icon-180', 'apple-touch-icon.png', app, 'image/png', 180, 180, 'fixed-dark-background'],
    ['favicon-ico', 'favicon.ico', ico, 'image/x-icon', 48, 48, 'fixed-dark-background'],
  ];
  for (const [, name, bytes] of outputs) if (name !== 'brand-symbol.svg') await writeFile(`${assetDir}/${name}`, bytes);
  const manifest = {
    purpose: 'Code-native A00 brand localization and deterministic local rasterization; not a visual-baseline or E04 approval',
    decision: 'DEC-045', license: 'unconfirmed', redistributable: 'pending', public_release: 'not-approved',
    source: { url: 'https://sb.sb.sb/assets/index.svg', retrieved_utc: '2026-09-04T20:35:34.074511+00:00', evidence: '.local/a00-redo/reference-index.svg', sha256: '3ac1426ddaa8208ef0c5bc83ca2432d9eb79f5de87c458283cab016262137f42', width: 512, height: 512, viewBox: '0 0 200 200', polygon_points: '6.5,6.5 193.5,6.5 193.5,76 177.5,100 193.5,124 193.5,193.5 6.5,193.5 6.5,124 22.5,100 6.5,76', stroke_width: 13, light: '#0a0a0a', dark: '#ffffff' },
    method: 'Keep the observed seal polygon coordinates, stroke-width 13 and light/dark colors exactly. Remove the complete original glyph path group. Add only 草莓 and 论坛 as two system-font rows in the same central region. Render the new SVG at each target size with local Chrome; no reference PNG is edited or used as pixel input.',
    source_background_observations: {
      favicon_16: { sha256: '6f6945b19cbc12d3cef76f34b96faea35e697cf1a692f266bfc50b13e8e47bf3', dimensions: [16, 16], alpha_extrema: [5, 255], corner_alpha: [6, 5, 6, 5] },
      favicon_32: { sha256: 'ca3ed9fd9697edd9512b026e57f5d775332f6f0b53cc2d45c7d31df0ff174037', dimensions: [32, 32], alpha_extrema: [0, 255], corner_alpha: [0, 0, 0, 0] },
      app_180: { sha256: '543420c0bb95d53cd0748317341a72e226b368d7c83f4a67aa6a12d28c6a2f30', dimensions: [180, 180], corner_rgba: [10, 10, 10, 255], white_seal_extent: [30, 30, 150, 150] },
      ico: { sha256: 'aaf15ba528a6236f926da1618bc66f3c0679f7072ba1e016b22b0f6c2d195228', format: '32-bit DIB', dimensions: [[16, 16], [32, 32], [48, 48]] },
      interpretation: 'Reference raster icons use a fixed #0a0a0a background with a white seal. App icon has a square canvas and a centered 120px seal. Favicon rounded-canvas radius 20% and seal inset 10% are geometric reconstruction from the observed raster edge positions, not captured vector declarations; native-Chrome edge antialiasing can differ from the source raster.',
    },
    renderer: { command: 'node scripts/a00-brand-render.mjs', node: process.version, browser: version.Browser, javascript: false, method: 'CDP diagnostic evaluation serializes the locally authored inline SVG into a data SVG image and draws it directly to an explicit sRGB canvas; PNG output comes from canvas.toDataURL. This avoids host-display color conversion in page screenshots.', color_space: 'srgb', page_network_requests: externalRequests, local_data_image_decodes: requests.length, scale: 1, zoom: 1, language: 'zh-CN', os: { platform: platform(), arch: arch(), kernel: release() }, fonts: uniqueFonts, font_note: 'Uses the installed OS font only; no font binary is copied or redistributed. Reproduction on another OS/font build is not guaranteed.', script_sha256: hash(await readFile('scripts/a00-brand-render.mjs')) },
    outputs: outputs.map(([logical_name, name, bytes, mime, width, height, theme]) => ({ logical_name, path: `${assetDir}/${name}`, sha256: hash(bytes), bytes: bytes.length, mime, width, height, theme, accessible_name: '草莓论坛', source: 'Localized current public reference seal; target system-font text and code-native raster generation', license: 'unconfirmed', redistributable: 'pending', public_release: 'not-approved', decision: 'DEC-045', evidence: [evidencePath, 'docs/visual/a00/brand-localization.md'] })),
    renders: renderRecords,
    limits: ['DEC-045 permits this local development/versioning step but is not a third-party license.', 'The original glyph paths are absent; four Chinese glyphs necessarily have different outlines. Small favicon text readability and six-viewport visual/WCAG gates remain for E04.', 'These generated raster icons are fixed dark-background variants. The standalone SVG retains the observed adaptive light/dark paint rules.'],
  };
  await writeFile(evidencePath, JSON.stringify(manifest, null, 2) + '\n');
  const sourceNames = { 'brand-symbol': 'index.svg', 'favicon-16': 'favicon-16x16.png', 'favicon-32': 'favicon-32x32.png', 'app-icon-180': 'apple-touch-icon.png', 'favicon-ico': 'favicon.ico' };
  const metadata = manifest.outputs.map(row => ({
    logical_name: row.logical_name, path: row.path, sha256: row.sha256,
    content_hash_filename: `${row.sha256}.${row.path.split('.').at(-1)}`,
    mime: row.mime, width: row.width, height: row.height, theme: row.theme,
    accessible_name: row.accessible_name, source: row.source,
    source_url: row.logical_name === 'favicon-ico' ? 'https://sb.sb/favicon.ico' : `https://sb.sb.sb/assets/${sourceNames[row.logical_name]}`,
    license: 'unconfirmed', redistributable: 'pending', local_versioning_decision: 'DEC-045', public_release: 'not-approved',
    evidence: [evidencePath, 'docs/visual/a00/brand-localization.md', 'plan/06-decisions-and-risks.md'],
    modification: manifest.method,
    ...(row.logical_name === 'favicon-ico' ? { sizes: [16, 32, 48].map(size => ({ width: size, height: size })), encoding: '40-byte BITMAPINFOHEADER, BI_RGB, 32bpp bottom-up BGRA XOR plus 1bpp AND mask; no palette' } : {}),
  }));
  await writeFile(metadataPath, JSON.stringify(metadata, null, 2) + '\n');
  if (process.argv.includes('--previews')) {
    const previewDir = '.local/a00-redo/brand-preview';
    await mkdir(previewDir, { recursive: true });
    const previews = [];
    for (const theme of ['light', 'dark']) {
      const png = await render(512, 'preview', theme), path = `${previewDir}/brand-symbol-${theme}.png`;
      await writeFile(path, png); previews.push({ path, sha256: hash(png), width: 512, height: 512, theme });
    }
    const path = `${previewDir}/favicon-48-frame.png`, png = iconPixels.get(48);
    await writeFile(path, png); previews.push({ path, sha256: hash(png), width: 48, height: 48, theme: 'fixed-dark-background' });
    await writeFile(`${previewDir}/manifest.json`, JSON.stringify({ purpose: 'Ignored local review/OCR previews only, not a visual baseline', source: sourcePath, source_sha256: hash(svg), browser: version.Browser, fonts: uniqueFonts, previews }, null, 2) + '\n');
  }
  console.log(JSON.stringify({ outputs: manifest.outputs.map(x => ({ path: x.path, sha256: x.sha256 })), fonts: uniqueFonts, deterministic_renders: renderRecords.length, external_requests: externalRequests.length }, null, 2));
} finally { await call('Target.disposeBrowserContext', { browserContextId }); socket.close(); }
