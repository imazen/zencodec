#!/usr/bin/env python3
"""Apply the reuse experiment to an existing PR183 prepared export, never a checkout."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('export', type=Path)
args = parser.parse_args()
root = args.export.resolve()
patches = Path(__file__).resolve().parent
if (root / '.git').exists() or (root / '.jj').exists():
    raise SystemExit('use an isolated prepared export')
expected = json.loads((patches / 'base-hashes.json').read_text())
for name, digest in expected.items():
    if hashlib.sha256((root / name).read_bytes()).hexdigest() != digest:
        raise SystemExit(f'base mismatch: {name}')
lock = root / 'core/perf/jpeg-policy-validation/Cargo.lock'
lock_hash = hashlib.sha256(lock.read_bytes()).hexdigest()
# Validate both before changing either. The accepted inputs are exact hashes.
for dry in [True, False]:
    for patch, directory in [('jpeg.patch', 'native/zenjpeg'), ('controls.patch', 'core')]:
        with (patches / patch).open() as source:
            subprocess.run(['patch', '--batch', '--fuzz=0', '-p1', *(['--dry-run'] if dry else [])],
                           cwd=root / directory, stdin=source, check=True, timeout=10)
assert hashlib.sha256(lock.read_bytes()).hexdigest() == lock_hash
(root / 'reuse-identity.json').write_text(json.dumps({
    'base_hashes': expected,
    'result_hashes': {name: hashlib.sha256((root / name).read_bytes()).hexdigest() for name in expected},
    'lock_sha256': lock_hash,
    'patch_sha256': {name: hashlib.sha256((patches / name).read_bytes()).hexdigest()
                     for name in ['jpeg.patch', 'controls.patch']},
}, indent=2) + '\n')
