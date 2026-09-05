#!/usr/bin/env python3
"""A01 URL-only wiring check against the original, unchanged A00 freeze."""
import hashlib
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def normalize_asset_helpers(text):
    # The only permitted transformation is one literal resource URL -> one helper.
    # Reversing it must recover the ENTIRE original template byte-for-byte.
    manifest = json.loads((ROOT / 'docs/visual/a00/assets.json').read_text())
    assets = manifest['assets'] if isinstance(manifest, dict) else manifest
    for asset in assets:
        logical = asset['path'].removeprefix('apps/web/static/')
        text = text.replace('{{ "' + logical + '"|asset_url }}', '/__fixtures/a00/assets/' + Path(logical).name)
    return text


def check():
    freeze = json.loads((ROOT / 'docs/visual/a00/structure-freeze.json').read_text())
    changed = []
    for name, expected in freeze['files'].items():
        data = (ROOT / name).read_bytes()
        if hashlib.sha256(data).hexdigest() == expected:
            continue
        if name.endswith('.html'):
            restored = normalize_asset_helpers(data.decode()).encode()
            if hashlib.sha256(restored).hexdigest() == expected:
                changed.append(name)
                continue
        raise AssertionError(f'A00 freeze drift beyond asset URL wiring: {name}')
    # A05 adds its explicitly scoped status page; the complete original A00 inventory
    # and every original byte/structure stay frozen. Other additions still fail closed.
    additions = {'apps/web/templates/status/index.html'}
    inventory = {p.relative_to(ROOT).as_posix() for p in (ROOT / 'apps/web/templates').rglob('*.html')}
    assert inventory == set(freeze['template_tag_order']) | additions
    original_classes = set()
    for name in freeze['template_tag_order']:
        for value in re.findall(r'class="([^"]*)"', (ROOT / name).read_text()):
            original_classes.update(value.split())
    for name in additions:
        for value in re.findall(r'class="([^"]*)"', (ROOT / name).read_text()):
            assert set(value.split()) <= original_classes, f'A05 introduced unreviewed component classes: {name}'
    return {'status': 'pass', 'url_only_templates': changed, 'original_freeze_unchanged': True,
            'scope': 'whole-source hashes after exact helper reversal; no visual gate claim'}


if __name__ == '__main__':
    print(json.dumps(check(), ensure_ascii=False, indent=2))
