#!/usr/bin/env python3
"""A00 source/SSR checks. stdlib only; deliberately not a visual release gate."""
import argparse
import hashlib
import json
import re
import sys
import unicodedata
import urllib.error
import urllib.request
from pathlib import Path
from asset_freeze import normalize_asset_helpers

ROOT = Path(__file__).resolve().parents[1]
DOC = ROOT / 'docs/visual/a00'
PAGES = json.loads((DOC / 'fixtures.json').read_text())['pages']
parser = argparse.ArgumentParser()
parser.add_argument('--base-url', default='http://127.0.0.1:18088')
parser.add_argument('--freeze', action='store_true', help='Explicitly write the reviewed development structure/class freeze inventory')
args = parser.parse_args()
errors = []
page_declaration = re.search(r'pub const PAGES:[^=]+?=\s*\[(.*?)\];', (ROOT / 'apps/web/src/fixtures.rs').read_text(), re.S)
if page_declaration is None or re.findall(r'"([^"]+)"', page_declaration.group(1)) != PAGES:
    errors.append('Rust page allowlist and fixtures.json differ')
terms = json.loads((DOC / 'old-names.json').read_text())['terms']
normalize = lambda s: unicodedata.normalize('NFKC', s).casefold()
terms = [normalize(s) for s in terms]
source = sorted((ROOT / 'apps/web/templates').rglob('*.html')) + sorted((ROOT / 'apps/web/static/css').rglob('*.css')) + sorted((ROOT / 'apps/web/static/img').rglob('*.svg')) + [ROOT / 'apps/web/src/fixtures.rs']
file_hashes = {}
classes = set()
template_structure = {}

def check_text(text, path):
    normalized = normalize(text)
    for term in terms:
        if term in normalized:
            errors.append(f'{path}: legacy name {term}')
    # XML namespace is a vocabulary identifier, not a resource request.
    runtime = re.sub(r'xmlns(?::\w+)?=["\'][^"\']+["\']', '', text)
    if re.search(r'(?:https?:)?//[a-z0-9]|(?:javascript|data):|@import|\b(?:fetch|XMLHttpRequest|WebSocket)\s*\(', runtime, re.I):
        errors.append(f'{path}: external or embedded runtime resource')
    for pattern in [r'-----BEGIN .*PRIVATE KEY', r'\b(?:AKIA|ASIA)[A-Z0-9]{16}\b', r'\bgh[pousr]_[A-Za-z0-9]{20,}', r'\b\d{1,3}(?:\.\d{1,3}){3}\b', r'[\w.+-]+@[\w.-]+\.[a-z]{2,}', r'\b(?:sessionid|csrftoken|authorization|set-cookie)\s*[:=]']:
        # Compact SVG path decimals can look like dotted IPv4 values. Their numeric
        # geometry is reviewed separately; retain full text for every other scan.
        candidate = re.sub(r'\b(?:d|points)="[^"]*"', '', text) if str(path).endswith('.svg') and 'd{1,3}' in pattern else text
        if re.search(pattern, candidate, re.I):
            errors.append(f'{path}: sensitive-value candidate ({pattern})')

for path in source:
    rel = path.relative_to(ROOT).as_posix()
    check_text(rel + '\n' + path.read_text(), rel)
    text = path.read_text()
    # A01 permits only exact URL -> helper wiring. Reverse just that substitution
    # before comparing the complete original bytes; never reset the A00 freeze.
    frozen_text = normalize_asset_helpers(text) if path.suffix == '.html' and not args.freeze else text
    file_hashes[rel] = hashlib.sha256(frozen_text.encode()).hexdigest()
    if path.suffix == '.html':
        # Ignore template control expressions before collecting literal CSS classes.
        clean = re.sub(r'{[%{].*?[%}]}', '', text, flags=re.S)
        for value in re.findall(r'class="([^"]*)"', clean):
            classes.update(value.split())
        template_structure[rel] = re.findall(r'<(/?[a-z][\w-]*)\b', clean)
    if '\r' in text or not text.endswith('\n'):
        errors.append(f'{rel}: UTF-8/LF/final newline')

freeze = {'status': 'a00-template-structure-v1 development freeze; package status in plan/99, not visual-baseline-v1', 'fixture_version': 'a00-synthetic-v1', 'files': file_hashes, 'template_tag_order': template_structure, 'literal_classes': sorted(classes)}
freeze_path = DOC / 'structure-freeze.json'
if args.freeze:
    freeze_path.write_text(json.dumps(freeze, ensure_ascii=False, indent=2) + '\n')
elif not freeze_path.exists() or json.loads(freeze_path.read_text()) != freeze:
    errors.append('frozen structure/CSS inventory drift; review before --freeze')

manifest = json.loads((DOC / 'assets.json').read_text())
assets = manifest['assets'] if isinstance(manifest, dict) else manifest
asset_paths = []
for asset in assets:
    asset_paths.append('/__fixtures/a00/assets/' + Path(asset['path']).name)
expected = [(f'/__fixtures/a00/{p}', 200) for p in PAGES] + [(p, 200) for p in asset_paths] + [('/__fixtures/a00/unknown', 404)]
smoke = dict(line.split() for line in (ROOT / 'scripts/smoke-routes.txt').read_text().splitlines() if line.strip() and not line.startswith('#'))
for path, status in expected:
    if smoke.get(path) != str(status):
        errors.append(f'smoke routes missing {path} {status}')
responses = []
asset_responses = []
for asset, path in zip(assets, asset_paths):
    try:
        with urllib.request.urlopen(args.base_url + path, timeout=10) as response:
            data, mime = response.read(), response.headers.get_content_type()
            digest = hashlib.sha256(data).hexdigest()
            if digest != asset['sha256'] or mime != asset['mime']:
                errors.append(f'{path}: served asset differs from source manifest; rebuild local SSR')
            if response.headers.get('Cache-Control') != 'no-store' or response.headers.get('X-Content-Type-Options') != 'nosniff':
                errors.append(f'{path}: source asset response headers differ from fixture contract')
            asset_responses.append({'path': path, 'sha256': digest, 'mime': mime})
    except (OSError, urllib.error.HTTPError) as e:
        errors.append(f'{path}: {e}')
for page in PAGES:
    for theme in ['light', 'dark', 'system']:
        path = f'/__fixtures/a00/{page}?theme={theme}'
        try:
            with urllib.request.urlopen(args.base_url + path, timeout=10) as first:
                data, mime = first.read(), first.headers.get_content_type()
            with urllib.request.urlopen(args.base_url + path, timeout=10) as second:
                again = second.read()
            if data != again or mime != 'text/html':
                errors.append(f'{path}: unstable bytes or wrong MIME')
            check_text(data.decode(), path)
            responses.append({'path': path, 'sha256': hashlib.sha256(data).hexdigest(), 'deterministic': data == again})
        except (OSError, urllib.error.HTTPError) as e:
            errors.append(f'{path}: {e}')
            break
result = {'command': 'python3 scripts/a00-check.py', 'status': 'pass' if not errors else 'fail', 'scope': f'development source + {len(PAGES) * 3} SSR variants + served asset hashes; no E04 pixel/WCAG claim', 'files_scanned': len(source), 'runtime_old_names': 'pass' if not errors else 'see errors', 'responses': responses, 'asset_responses': asset_responses, 'errors': errors}
(DOC / 'checks.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
print(json.dumps({'status': result['status'], 'files': len(source), 'responses': len(responses), 'errors': errors}, ensure_ascii=False, indent=2))
sys.exit(bool(errors))
