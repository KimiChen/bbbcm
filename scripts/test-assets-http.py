#!/usr/bin/env python3
"""A01 on-demand loopback HTTP checks against the ACTUAL dist; no asset build or DB integration."""
import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
DIST = ROOT / 'apps/web/static/dist'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    manifest_bytes = (DIST / 'manifest.json').read_bytes()
    manifest = json.loads(manifest_bytes)
    by_url = {'/assets/' + a['path']: a for a in manifest['assets']}
    baseline = {r['path']: r['sha256'] for r in json.loads((ROOT / 'docs/visual/a00/checks.json').read_text())['responses']}
    pages = json.loads((ROOT / 'docs/visual/a00/fixtures.json').read_text())['pages']
    checks = 0
    variants = 0
    startup_failures = 0
    for environment in ['development', 'test', 'staging', 'production']:
        with tempfile.TemporaryDirectory(prefix='bbbcm-assets-http-') as directory:
            temp = Path(directory)
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', 0))
                port = sock.getsockname()[1]
            config = temp / 'config.toml'
            config.write_text(f'''[server]
environment = "{environment}"
bind = "127.0.0.1:{port}"
[database]
backend = "sqlite"
schema_prefix = "bcm_"
sqlite_path = "{temp / 'unused.db'}"
max_connections = 1
[features]
redis = {str(environment in ['staging', 'production']).lower()}
[worker]
embedded = false
''')
            env = {k: v for k, v in os.environ.items() if not k.startswith('BBBCM_')}
            env.update(BBBCM_CONFIG=str(config), BBBCM_ASSET_DIST=str(DIST), RUST_LOG='info')
            command = ['cargo', 'run', '--quiet', '--locked', '--package', 'bbbcm-web', '--']
            log = temp / 'web.log'
            with log.open('w') as output:
                process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=output, stderr=subprocess.STDOUT)
                try:
                    deadline = time.monotonic() + 120
                    while 'bbbcm-web 监听' not in log.read_text():
                        assert process.poll() is None, 'HTTP test process failed to start; inspect local startup configuration'
                        assert time.monotonic() < deadline, 'HTTP test startup timed out'
                        time.sleep(0.1)

                    def request(url, want=200, method='GET'):
                        nonlocal checks
                        connection = http.client.HTTPConnection('127.0.0.1', port, timeout=5)
                        try:
                            connection.request(method, url)
                            response = connection.getresponse()
                            data = response.read()
                            assert response.status == want, (environment, url, response.status, want)
                            checks += 1
                            return dict(response.getheaders()), data
                        finally:
                            connection.close()

                    for url, entry in by_url.items():
                        headers, data = request(url)
                        assert hashlib.sha256(data).hexdigest() == entry['sha256']
                        assert len(data) == entry['size']
                        assert headers['content-type'] == entry['mime']
                        assert headers['cache-control'] == 'public, max-age=31536000, immutable'
                        assert headers['x-content-type-options'] == 'nosniff'
                        headers, data = request(url, method='HEAD')
                        assert data == b'' and int(headers['content-length']) == entry['size']
                        assert headers['cache-control'] == 'public, max-age=31536000, immutable'
                    for url in ['/assets/missing.css', '/assets/manifest.json', '/assets/css/site.css']:
                        headers, _ = request(url, 404)
                        assert headers['cache-control'] == 'no-store'
                    for url in ['/assets', '/assets/', '/assets/../Cargo.toml', '/assets/%2e%2e%2f.env',
                                '/assets/%252e%252e%252f.env', '/assets/%00', '/assets/css%2fsite.css']:
                        headers, _ = request(url, 400)
                        assert headers['cache-control'] == 'no-store'
                    if environment in ['development', 'test']:
                        for page in pages:
                            for theme in ['light', 'dark', 'system']:
                                url = f'/__fixtures/a00/{page}?theme={theme}'
                                _, data = request(url)
                                assert data == request(url)[1]
                                assert b'/__fixtures/a00/assets/' not in data
                                assert by_url and any(u.encode() in data for u in by_url)
                                # Exact baseline SSR bytes after reversing ONLY asset URLs.
                                restored = data
                                for fingerprint, entry in by_url.items():
                                    restored = restored.replace(fingerprint.encode(),
                                        ('/__fixtures/a00/assets/' + Path(entry['logical']).name).encode())
                                assert hashlib.sha256(restored).hexdigest() == baseline[url], ('A00 SSR drift', url)
                                variants += 1
                        request('/__fixtures/a00/assets/site.css')
                    else:
                        for page in pages:
                            request('/__fixtures/a00/' + page, 404)
                        for url in ['/__fixtures/a00/assets/site.css', '/static/css/site.css', '/static/dist/manifest.json',
                                    '/img/brand-mark.svg', '/fonts/space-mono-latin-400-normal.woff2', '/.env', '/Cargo.toml']:
                            request(url, 404)
                    assert not (temp / 'unused.db').exists(), 'This is an HTTP-only asset test'
                finally:
                    process.terminate()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
            # Exercise real startup failures (not only the Rust loader in isolation).
            for kind in ['missing', 'malformed']:
                invalid = temp / kind
                if kind == 'malformed':
                    invalid.mkdir()
                    (invalid / 'manifest.json').write_text('{')
                env['BBBCM_ASSET_DIST'] = str(invalid)
                result = subprocess.run(command, cwd=ROOT, env=env, text=True, stdout=subprocess.PIPE,
                                        stderr=subprocess.STDOUT, timeout=30)
                assert result.returncode != 0 and '静态资产加载失败' in result.stdout
                assert 'bbbcm-web 监听' not in result.stdout
                startup_failures += 1
    result = {'status': 'pass', 'manifest_sha256': hashlib.sha256(manifest_bytes).hexdigest(),
              'assets': len(by_url), 'http_checks': checks, 'fingerprint_ssr_variants': variants,
              'startup_failures_correctly_rejected': startup_failures,
              'a00_original_ssr_bytes_after_url_reversal': 'pass',
              'environments': ['development', 'test', 'staging', 'production'],
              'scope': 'loopback HTTP assets and SSR; no real PostgreSQL/Redis, E2E browser or visual gate'}
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
