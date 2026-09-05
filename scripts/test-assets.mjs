import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, readdir, rm, mkdir, writeFile, symlink } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { buildAssets, compileAssets, cleanAssets, checkToolchain, sha256, SOURCE, MIME } from './build-assets.mjs';

await checkToolchain();
async function temporary(run) {
  const root = await mkdtemp(path.join(os.tmpdir(), 'bbbcm-assets-'));
  try { await run(root); } finally { await rm(root, { recursive: true, force: true }); }
}
async function put(root, name, text) {
  await mkdir(path.dirname(path.join(root, name)), { recursive: true });
  await writeFile(path.join(root, name), text);
}
async function allFiles(root, prefix = '') {
  const result = [];
  for (const item of await readdir(path.join(root, prefix), { withFileTypes: true })) {
    const name = prefix + item.name;
    if (item.isDirectory()) result.push(...await allFiles(root, name + '/'));
    else result.push(name);
  }
  return result.sort();
}
async function validateDist(dist, manifest) {
  assert.equal(manifest.version, 1);
  assert.deepEqual(await allFiles(dist), [...manifest.assets.map(a => a.path), 'manifest.json'].sort());
  const paths = new Set(manifest.assets.map(a => '/assets/' + a.path));
  let fonts = 0, images = 0;
  for (const entry of manifest.assets) {
    const bytes = await readFile(path.join(dist, entry.path));
    assert.equal(bytes.length, entry.size);
    assert.equal(sha256(bytes), entry.sha256);
    assert.ok(entry.path.includes('.' + entry.sha256 + '.'));
    assert.equal(MIME[path.extname(entry.path)], entry.mime);
    if (entry.logical.endsWith('.css')) {
      const text = bytes.toString('utf8');
      assert.ok(!text.includes('/__fixtures/') && !text.includes('sourceMappingURL'));
      for (const match of text.matchAll(/url\(\s*["']?([^\s"')]+)["']?\s*\)/g)) {
        const ref = match[1].split('#')[0];
        assert.ok(paths.has(ref), `Unresolved CSS reference: ${ref}`);
        if (ref.includes('/fonts/')) fonts++;
        if (ref.includes('/img/')) images++;
      }
      assert.doesNotMatch(text, /@import|https?:\/\/|url\(\s*["']?\/\//i);
    }
    if (entry.logical.endsWith('.svg')) {
      const text = bytes.toString('utf8').replace(/xmlns(?::\w+)?=["'][^"']+["']/g, '');
      assert.doesNotMatch(text, /(?:src|href)=["'](?:https?:|\/\/)|url\(\s*["']?(?:https?:|\/\/)/i);
    }
  }
  return { fonts, images };
}

test('actual A00 inventory: complete SHA-256 manifest, CSS references and byte-identical rebuild', async () => temporary(async root => {
  const first = await buildAssets(SOURCE, path.join(root, 'first'));
  assert.equal(first.assets.length, 30);
  const references = await validateDist(path.join(root, 'first'), first);
  assert.equal(references.fonts, 4);
  assert.equal(references.images, 11);
  for (const entry of first.assets.filter(a => /^(img|fonts)\//.test(a.logical))) {
    assert.deepEqual(await readFile(path.join(SOURCE, entry.logical)), await readFile(path.join(root, 'first', entry.path)));
  }
  const second = await buildAssets(SOURCE, path.join(root, 'second'));
  assert.deepEqual(first, second);
  for (const name of await allFiles(path.join(root, 'first'))) {
    assert.deepEqual(await readFile(path.join(root, 'first', name)), await readFile(path.join(root, 'second', name)), name);
  }
}));

async function synthetic(root) {
  const src = path.join(root, 'source');
  await put(src, 'img/icon.svg', '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"></svg>\n');
  await put(src, 'fonts/test.woff2', await readFile(path.join(SOURCE, 'fonts/space-mono-latin-400-normal.woff2')));
  await put(src, 'css/base.css', ':root { --space: 4px; }\n');
  await put(src, 'css/main.css', '@import "./base.css";\n@font-face { font-family: Test; src: url("../fonts/test.woff2"); }\n.item { padding: var(--space); mask: url("../img/icon.svg"); }\n');
  await put(src, 'js/helper.js', 'export const value = 2;\n');
  await put(src, 'js/main.ts', 'import { value } from "./helper.js";\nimport icon from "../img/icon.svg";\nconst count: number = value;\nexport { count, icon };\n');
  return src;
}

test('esbuild bundles CSS imports, TypeScript, JS imports and local binary exports; hashes propagate', async () => temporary(async root => {
  const src = await synthetic(root);
  const dist = path.join(root, 'dist');
  const before = await buildAssets(src, dist);
  await validateDist(dist, before);
  const byName = new Map(before.assets.map(a => [a.logical, a]));
  const js = await readFile(path.join(dist, byName.get('js/main.ts').path), 'utf8');
  assert.match(js, /export\s*\{/);
  assert.ok(js.includes('/assets/' + byName.get('img/icon.svg').path));
  assert.doesNotMatch(js, /: number|\.\/helper\.js|sourceMappingURL/);
  const css = await readFile(path.join(dist, byName.get('css/main.css').path), 'utf8');
  assert.ok(css.includes('--space: 4px') && !css.includes('@import'));
  await put(src, 'img/icon.svg', '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"></svg>\n');
  const after = await buildAssets(src, dist);
  await validateDist(dist, after);
  for (const entry of after.assets) {
    const changed = ['img/icon.svg', 'css/main.css', 'js/main.ts'].includes(entry.logical);
    assert.equal(entry.sha256 !== byName.get(entry.logical).sha256, changed, entry.logical);
  }
}));

test('remote CSS/imports, unknown/missing dependencies and code warnings fail without replacing old dist', async () => temporary(async root => {
  const src = await synthetic(root);
  const dist = path.join(root, 'dist');
  await buildAssets(src, dist);
  const before = await readFile(path.join(dist, 'manifest.json'));
  for (const css of [
    '@import "https://invalid.example/style.css";',
    '.x { background: url(//invalid.example/pixel.png); }',
    '.x { background: url(https://invalid.example/pixel.png); }',
    '.x { background: url(../img/missing.png); }',
    '.x { background: url(../../../secret.png); }',
    '.x { background: url(data:image/png;base64,AA==); }',
    '.x { color: red; ',
  ]) {
    await put(src, 'css/main.css', css);
    await assert.rejects(buildAssets(src, dist));
    assert.deepEqual(await readFile(path.join(dist, 'manifest.json')), before);
  }
  await put(src, 'css/main.css', '.x { color: red; }');
  for (const js of ['import "https://invalid.example/code.js";', 'import("//invalid.example/code.js");', 'import "../css/main.css";']) {
    await put(src, 'js/main.ts', js);
    await assert.rejects(buildAssets(src, dist));
  }
  await put(src, 'img/private.txt', 'unsupported');
  await assert.rejects(compileAssets(src), /Unsupported asset/);
}));

test('symlink source and output cannot escape or expose other files', async () => temporary(async root => {
  const src = await synthetic(root);
  await symlink(path.join(src, 'img/icon.svg'), path.join(src, 'img/linked.svg'));
  await assert.rejects(compileAssets(src), /symlink/);
  await rm(path.join(src, 'img/linked.svg'));
  await symlink(src, path.join(root, 'linked-source'));
  await assert.rejects(compileAssets(path.join(root, 'linked-source')), /symlink/);
  await symlink(src, path.join(root, 'dist'));
  await assert.rejects(buildAssets(src, path.join(root, 'dist')), /regular directory/);
  await assert.rejects(cleanAssets(path.join(root, 'dist')), /regular directory/);
  assert.ok((await readFile(path.join(src, 'img/icon.svg'))).length > 0);
}));

test('clean is repeatable, scoped to dist, and never steals a concurrent build lock', async () => temporary(async root => {
  const src = await synthetic(root);
  const dist = path.join(root, 'dist');
  await buildAssets(src, dist);
  await mkdir(path.join(root, '.dist-lock'));
  await assert.rejects(buildAssets(src, dist), /EEXIST/);
  await assert.rejects(cleanAssets(dist), /EEXIST/);
  await rm(path.join(root, '.dist-lock'), { recursive: true });
  await cleanAssets(dist);
  await cleanAssets(dist);
  assert.ok((await readFile(path.join(src, 'img/icon.svg'))).length > 0);
}));
