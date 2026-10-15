#!/usr/bin/env python3
"""Check the static Pages payload before it is uploaded (no dependencies)."""
from pathlib import Path
from html.parser import HTMLParser
from urllib.parse import unquote, urlsplit
import re

root = Path(__file__).resolve().parent.parent / 'web'
errors = []


def check_url(source, url):
    parts = urlsplit(url)
    if parts.scheme or parts.netloc or not parts.path:
        return
    if parts.path.startswith('/'):
        errors.append(f'{source.relative_to(root)}: site-root URL breaks project Pages: {url}')
        return
    target = source.parent / unquote(parts.path)
    if not target.exists():
        errors.append(f'{source.relative_to(root)}: missing {url}')


class Assets(HTMLParser):
    def handle_starttag(self, tag, attrs):
        for name, value in attrs:
            if name in ('src', 'href') and value:
                check_url(self.source, value)


for source in root.rglob('*.html'):
    parser = Assets()
    parser.source = source
    parser.feed(source.read_text())

for name in ('app.js', 'worker.js', 'lsp.js', 'pkg/scas.js'):
    source = root / name
    if not source.exists():
        errors.append(f'Missing {name}')
        continue
    for url in re.findall(r'''["'](\./[^"']+\.(?:js|wasm))["']''', source.read_text()):
        check_url(source, url)

wasm = root / 'pkg/scas_bg.wasm'
if not wasm.exists() or wasm.read_bytes()[:8] != b'\0asm\x01\0\0\0':
    errors.append('Missing or invalid WebAssembly module')
if not (root / '.nojekyll').exists():
    errors.append('Missing .nojekyll')
if errors:
    raise SystemExit('\n'.join(errors))
print(f'Pages payload verified: local links, module assets and Wasm ({wasm.stat().st_size:,} bytes).')
