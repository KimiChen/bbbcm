// WP-MVP-A01. No downloads, floating tools, timestamps, source maps or source edits.
import { createHash } from 'node:crypto';
import { lstat, mkdir, mkdtemp, readFile, readdir, realpath, rename, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import * as esbuild from 'esbuild';

export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const SOURCE = path.join(ROOT, 'apps/web/static');
export const DIST = path.join(SOURCE, 'dist');
export const MIME = Object.freeze({
  '.css': 'text/css; charset=utf-8', '.js': 'text/javascript; charset=utf-8',
  '.svg': 'image/svg+xml', '.png': 'image/png', '.ico': 'image/x-icon',
  '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.webp': 'image/webp', '.gif': 'image/gif',
  '.woff': 'font/woff', '.woff2': 'font/woff2', '.ttf': 'font/ttf', '.otf': 'font/otf',
});
const categories = ['css', 'js', 'img', 'fonts'];
const code = name => /\.(css|js|ts)$/.test(name);
export const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const require = (ok, message) => { if (!ok) throw new Error(message); };
const exists = async file => {
  try { return await lstat(file); } catch (error) { if (error.code === 'ENOENT') return null; throw error; }
};

export async function checkToolchain() {
  const pkg = JSON.parse(await readFile(path.join(ROOT, 'package.json'), 'utf8'));
  require(process.versions.node === pkg.engines.node, 'Use scripts/assets.py with the M0-03 locked Node');
  require(esbuild.version === pkg.devDependencies.esbuild, 'esbuild version differs from the shared lock');
}

async function inventory(source) {
  const files = new Map();
  async function walk(logical) {
    const file = path.join(source, logical);
    const stat = await lstat(file);
    require(!stat.isSymbolicLink(), `Source symlink rejected: ${logical}`);
    if (path.basename(logical) === '.gitkeep' && stat.isFile() && stat.size === 0) return;
    require(logical.split('/').every(p => /^[a-z0-9][a-z0-9._-]*$/.test(p)), `Invalid asset name: ${logical}`);
    if (stat.isDirectory()) {
      for (const name of (await readdir(file)).sort()) await walk(`${logical}/${name}`);
      return;
    }
    require(stat.isFile(), `Not a regular asset: ${logical}`);
    const ext = path.extname(logical);
    const category = logical.split('/')[0];
    require(category === 'css' ? ext === '.css' : category === 'js' ? ['.js', '.ts'].includes(ext) :
      category === 'img' ? MIME[ext]?.startsWith('image/') : MIME[ext]?.startsWith('font/'),
    `Unsupported asset: ${logical}`);
    require(stat.size > 0 && stat.size <= 16 * 1024 * 1024, `Asset size out of bounds: ${logical}`);
    files.set(logical, await readFile(file));
  }
  require(!(await lstat(source)).isSymbolicLink(), 'Source root must not be a symlink');
  for (const category of categories) if (await exists(path.join(source, category))) await walk(category);
  require(files.size > 0 && files.size <= 4096, 'Asset inventory is empty or too large');
  return files;
}

// Binary bytes remain identical. Esbuild resolves CSS URLs and JS file imports to
// these SHA-256 names before compiling code, so code hashes include dependency URLs.
export async function compileAssets(source) {
  const inputs = await inventory(source);
  // macOS /var is an ancestor alias for /private/var. Match esbuild's canonical
  // paths after rejecting actual symlinks inside the source inventory.
  source = await realpath(source);
  const entries = new Map();
  const outputs = new Map();
  function emit(logical, bytes) {
    require(bytes.length > 0 && bytes.length <= 16 * 1024 * 1024, `Output size out of bounds: ${logical}`);
    const extension = path.extname(logical) === '.ts' ? '.js' : path.extname(logical);
    const stem = logical.slice(0, -path.extname(logical).length);
    const digest = sha256(bytes);
    const file = `${stem}.${digest}${extension}`;
    require(!outputs.has(file), `Output collision: ${logical}`);
    outputs.set(file, bytes);
    const entry = { logical, path: file, sha256: digest, size: bytes.length, mime: MIME[extension] };
    entries.set(logical, entry);
    return entry;
  }
  for (const [logical, bytes] of inputs) if (!code(logical)) emit(logical, bytes);

  const plugin = {
    name: 'local-content-addressed-assets',
    setup(build) {
      build.onResolve({ filter: /.*/ }, args => {
        if (args.kind === 'entry-point') return;
        if (args.kind === 'url-token' && /^#[a-zA-Z0-9_-]+$/.test(args.path)) return { path: args.path, external: true };
        let logical;
        let fragment = '';
        const reference = args.path.replace(/#[a-zA-Z0-9_-]+$/, value => { fragment = value; return ''; });
        if (reference.startsWith('/__fixtures/a00/assets/')) {
          const name = reference.slice('/__fixtures/a00/assets/'.length);
          const matches = [...inputs.keys()].filter(key => path.posix.basename(key) === name);
          require(matches.length === 1, `Missing or ambiguous A00 asset: ${reference}`);
          logical = matches[0];
        } else {
          require(!/^(?:[a-z][a-z0-9+.-]*:|\/|\\)/i.test(reference) && !reference.includes('?') &&
            !reference.includes('\\'), `Non-local resource rejected: ${args.path}`);
          require(args.kind === 'url-token' || reference.startsWith('.'), `Bare import rejected: ${reference}`);
          const resolved = path.resolve(args.resolveDir, reference);
          logical = path.relative(source, resolved).split(path.sep).join('/');
          // Explicit extensions keep the allowed input graph deterministic.
          require(inputs.has(logical), `Missing or out-of-root asset: ${reference}`);
        }
        if (args.kind === 'url-token') {
          const entry = entries.get(logical);
          require(entry, `CSS URL must reference an image or font: ${logical}`);
          return { path: `/assets/${entry.path}${fragment}`, external: true };
        }
        require(!fragment, 'Import fragments are not supported');
        if (!code(logical)) return { path: logical, namespace: 'fingerprint' };
        require(!(logical.endsWith('.css') && !args.importer.endsWith('.css')),
          'Link CSS through the SSR helper; JS must not hide a stylesheet dependency');
        return { path: path.join(source, logical) };
      });
      build.onLoad({ filter: /.*/, namespace: 'fingerprint' }, args => ({
        contents: `export default ${JSON.stringify(`/assets/${entries.get(args.path).path}`)};`, loader: 'js',
      }));
      build.onLoad({ filter: /\.(css|js|ts)$/ }, args => {
        const logical = path.relative(source, args.path).split(path.sep).join('/');
        require(inputs.has(logical), `Unlisted code input: ${logical}`);
        return { contents: inputs.get(logical).toString('utf8'), loader: path.extname(logical).slice(1),
          resolveDir: path.dirname(args.path) };
      });
    },
  };
  for (const logical of inputs.keys()) {
    if (!code(logical)) continue;
    const result = await esbuild.build({
      absWorkingDir: source, entryPoints: [logical], bundle: true, write: false,
      outfile: `output${logical.endsWith('.css') ? '.css' : '.js'}`, plugins: [plugin],
      platform: 'browser', format: 'esm', target: 'es2022', charset: 'utf8',
      minify: false, sourcemap: false, legalComments: 'inline', metafile: true, logLevel: 'silent',
    });
    require(result.warnings.length === 0, `esbuild warnings in ${logical}: ${result.warnings.map(w => w.text).join('; ')}`);
    require(result.outputFiles.length === 1, `Unexpected side output: ${logical}`);
    for (const output of Object.values(result.metafile.outputs)) for (const ref of output.imports) {
      require(ref.external && (ref.path.startsWith('/assets/') || ref.path.startsWith('#')),
        `Unresolved output dependency: ${ref.path}`);
    }
    emit(logical, result.outputFiles[0].contents);
  }
  const assets = [...entries.values()].sort((a, b) => a.logical < b.logical ? -1 : 1);
  require(assets.reduce((sum, a) => sum + a.size, 0) <= 128 * 1024 * 1024, 'Asset snapshot exceeds 128 MiB');
  return { manifest: { version: 1, assets }, outputs };
}

async function withOutputLock(dist, action) {
  await mkdir(path.dirname(dist), { recursive: true });
  const lock = path.join(path.dirname(dist), '.dist-lock');
  await mkdir(lock); // Concurrent builds/clean fail explicitly; no destructive lock stealing.
  try {
    const stat = await exists(dist);
    require(!stat || (stat.isDirectory() && !stat.isSymbolicLink()), 'dist must be a regular directory');
    return await action();
  } finally { await rm(lock, { recursive: true }); }
}

export async function buildAssets(source = SOURCE, dist = DIST) {
  return withOutputLock(dist, async () => {
    const compiled = await compileAssets(source); // Failure leaves the previous dist untouched.
    const stage = await mkdtemp(path.join(path.dirname(dist), '.dist-build-'));
    const previous = path.join(stage, 'previous');
    const next = path.join(stage, 'next');
    await mkdir(next);
    let preservePrevious = false;
    try {
      for (const [file, bytes] of compiled.outputs) {
        await mkdir(path.dirname(path.join(next, file)), { recursive: true });
        await writeFile(path.join(next, file), bytes);
      }
      await writeFile(path.join(next, 'manifest.json'), JSON.stringify(compiled.manifest, null, 2) + '\n');
      const hadPrevious = Boolean(await exists(dist));
      if (hadPrevious) await rename(dist, previous);
      try { await rename(next, dist); } catch (error) {
        if (hadPrevious) {
          try { await rename(previous, dist); } catch (restoreError) {
            preservePrevious = true;
            throw new Error(`Output replacement and restore failed; recover ${previous}`, { cause: restoreError });
          }
        }
        throw error;
      }
    } finally { if (!preservePrevious) await rm(stage, { recursive: true, force: true }); }
    return compiled.manifest;
  });
}

export async function cleanAssets(dist = DIST) {
  return withOutputLock(dist, () => rm(dist, { recursive: true, force: true }));
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    await checkToolchain();
    if (process.argv.length !== 3 || !['build', 'clean'].includes(process.argv[2])) throw new Error('Usage: build-assets.mjs build|clean');
    if (process.argv[2] === 'clean') { await cleanAssets(); console.log('Removed apps/web/static/dist'); }
    else { const result = await buildAssets(); console.log(`Built ${result.assets.length} fingerprint assets and manifest.json`); }
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
