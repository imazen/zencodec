#!/usr/bin/env python3
"""Keep standalone validation roots on the core's pixel/garb source identities.

Only shared interchange dependencies are synchronized. Codec and numerical
backend pins remain explicit in each root; prior receipts retain their sources.
"""
import argparse
from pathlib import Path
import re
import tomllib


def sync(root, write=False):
    root = Path(root)
    patches = tomllib.loads((root / 'Cargo.toml').read_text())['patch']['crates-io']
    selected = {name: patches[name] for name in ('zenpixels', 'zenpixels-convert', 'garb')}
    changed = []
    for manifest in sorted(root.rglob('Cargo.toml')):
        if set(manifest.relative_to(root).parts) & {'vendor', 'target', 'node_modules', '.git', '.jj'}:
            continue
        text = original = manifest.read_text()
        for name, source in selected.items():
            pattern = re.compile(r'(?m)^(\s*' + re.escape(name) + r'\s*=\s*\{[^\n]*git\s*=\s*"' + re.escape(source['git']) + r'"[^\n]*rev\s*=\s*")[a-f0-9]+(")')
            text = pattern.sub(lambda match: match[1] + source['rev'] + match[2], text)
        if text != original:
            changed.append(str(manifest.relative_to(root)))
            if write:
                manifest.write_text(text)
    return changed


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    changes = sync(Path(__file__).resolve().parents[1], args.write)
    for name in changes:
        print(('updated ' if args.write else 'stale ') + name)
    raise SystemExit(0 if args.write or not changes else 1)
