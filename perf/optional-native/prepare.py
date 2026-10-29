#!/usr/bin/env python3
"""Export frozen native sources and bridge only the shared core/pixel family."""
import argparse
import io
import json
from pathlib import Path
import re
import subprocess
import tarfile

FAMILY = ('zencodec', 'zencodec-io', 'zencodec-coroutine', 'zencodec-testkit',
          'zencodec-media', 'zencodec-media-testkit', 'zenmeta')
PIXELS = '283655c5424e0ad7ebc6d2acd94670fac1e29158'


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('repository', type=Path)
    p.add_argument('revision')
    p.add_argument('destination', type=Path)
    p.add_argument('core', type=Path)
    a = p.parse_args()
    if a.destination.exists():
        raise SystemExit('refusing to replace an existing export')
    sha = subprocess.check_output(['git', '-C', str(a.repository), 'rev-parse', a.revision], text=True).strip()
    data = subprocess.check_output(['git', '-C', str(a.repository), 'archive', sha])
    a.destination.mkdir(parents=True)
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        archive.extractall(a.destination, filter='data')
    manifest = a.destination / 'Cargo.toml'
    original = manifest.read_text()
    paths = {name: str(a.core.resolve() / ('' if name == 'zencodec' else name)) for name in FAMILY}
    lines = original.splitlines()
    for i, line in enumerate(lines):
        key = line.split('=', 1)[0].strip()
        if key in paths and 'github.com/imazen/zencodec' in line:
            # Keep feature/default-feature arguments on direct dependency edges.
            line = re.sub(r'git\s*=\s*"[^"]+"\s*,\s*rev\s*=\s*"[^"]+"',
                          'path = ' + json.dumps(paths[key]), line)
        if key in ('zenpixels', 'zenpixels-convert') and 'github.com/imazen/zenpixels' in line:
            line = re.sub(r'rev\s*=\s*"[^"]+"', 'rev = "' + PIXELS + '"', line)
        lines[i] = line
    text = '\n'.join(lines) + '\n'
    for section in ('patch.crates-io', 'patch."https://github.com/imazen/zencodec"'):
        header = '[' + section + ']'
        if header not in text:
            text += '\n' + header + '\n'
        start = text.index(header) + len(header)
        end = text.find('\n[', start)
        if end < 0:
            end = len(text)
        body = text[start:end]
        present = set(re.findall(r'^([\w-]+)\s*=', body, re.M))
        additions = ''.join(name + ' = { path = ' + json.dumps(path) + ' }\n'
                            for name, path in paths.items() if name not in present)
        if section == 'patch.crates-io':
            for name in ('zenpixels', 'zenpixels-convert'):
                if name not in present:
                    additions += name + ' = { git = "https://github.com/imazen/zenpixels", rev = "' + PIXELS + '" }\n'
        text = text[:end] + '\n' + additions + text[end:]
    manifest.write_text(text)
    lock = a.destination / 'Cargo.lock'
    if lock.exists():
        (a.destination / 'Cargo.lock.before').write_bytes(lock.read_bytes())
    (a.destination / 'OPTIONAL-SOURCE.json').write_text(json.dumps({
        'revision': sha, 'repository': str(a.repository), 'core': str(a.core.resolve()),
        'pixels': PIXELS, 'manifest_before': original, 'manifest_after': text,
        'scope': 'isolated source qualification; not portable dependency landing'
    }, indent=2) + '\n')
    print(sha, a.destination)


if __name__ == '__main__':
    main()
