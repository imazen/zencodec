#!/usr/bin/env python3
"""Fetch two immutable native witnesses and decode independent active samples."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import urllib.request

REVISION = '5340e44cc6a01a843b43533576948f138140f077'
FIXTURES = {
    'mono_5x3_8b_full.avif': ('9ba5dc9465ab9672818bd8989e74bf782f070eaba3716ee68301c91299b6083f', 'gray'),
    'mono_gradient_10b_full.avif': ('2694513f1a32997ab50f9f846dc39b57437611613a735aa7e32b7ac06a032d75', 'gray10le'),
}
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('output', type=Path)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
receipt = {'fixture_revision': REVISION, 'oracle_version': subprocess.check_output(['ffmpeg', '-version'], text=True).splitlines()[0], 'fixtures': []}
for name, (sha, fmt) in FIXTURES.items():
    path = args.output / name
    url = f'https://raw.githubusercontent.com/imazen/zenavif/{REVISION}/tests/vectors/zenavif/{name}'
    data = path.read_bytes() if path.exists() else urllib.request.urlopen(url, timeout=90).read()
    if hashlib.sha256(data).hexdigest() != sha:
        raise RuntimeError(f'fixture identity mismatch: {name}')
    if not path.exists():
        path.write_bytes(data)
    raw = args.output / (name + '.raw')
    command = ['ffmpeg', '-v', 'error', '-threads', '1', '-i', str(path), '-frames:v', '1', '-pix_fmt', fmt, '-f', 'rawvideo', 'pipe:1']
    reference = subprocess.check_output(command, stderr=subprocess.PIPE, timeout=90)
    if raw.exists() and raw.read_bytes() != reference:
        raise RuntimeError(f'existing oracle output differs: {raw}; preserve both runs in separate directories')
    if not raw.exists():
        raw.write_bytes(reference)
    receipt['fixtures'].append({'name': name, 'url': url, 'sha256': sha, 'reference_sha256': hashlib.sha256(reference).hexdigest(), 'reference_bytes': len(reference), 'command': command})
(args.output / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
