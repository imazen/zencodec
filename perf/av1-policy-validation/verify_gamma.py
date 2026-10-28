#!/usr/bin/env python3
"""Check saved native Gamma22 traces against the selected reflected power model."""
import argparse
import hashlib
import json
import math
import struct
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('artifacts', type=Path)
parser.add_argument('report', type=Path)
parser.add_argument('--presentations', type=int, default=54)
args = parser.parse_args()
inputs = sorted(args.artifacts.glob('planar-*/gamma-linear-*.raw'))
if len(inputs) != args.presentations:
    raise SystemExit(f'expected {args.presentations} traces, found {len(inputs)}')
records = []
for source in inputs:
    encoded = source.with_name(source.name.replace('gamma-linear-', 'gamma-encoded-'))
    restored = source.with_name(source.name.replace('gamma-linear-', 'gamma-restored-'))
    raw = [p.read_bytes() for p in (source, encoded, restored)]
    if not raw[0] or len(raw[0]) % 12 or len({len(v) for v in raw}) != 1:
        raise SystemExit(f'invalid RGB F32 trace lengths: {source}')
    data = [struct.unpack(f'<{len(v) // 4}f', v) for v in raw]
    max_error = max_roundtrip = 0.0
    extended = 0
    for x, y, z in zip(*data):
        expected = math.copysign(abs(x) ** (256 / 563), x)
        error, roundtrip = abs(y - expected), abs(z - x)
        if not all(map(math.isfinite, (x, y, z))):
            raise SystemExit(f'nonfinite sample: {source}: {x}, {y}, {z}')
        if error > 2e-5 * max(1, abs(expected)) or roundtrip > 2e-5 * max(1, abs(x)):
            raise SystemExit(f'power/roundtrip disagreement: {source}: {x}, {y}, {z}')
        if not 0 <= x <= 1:
            if 0 <= y <= 1 or math.copysign(1, y) != math.copysign(1, x):
                raise SystemExit(f'extended sample collapsed: {source}: {x}, {y}')
            extended += 1
        max_error = max(max_error, error)
        max_roundtrip = max(max_roundtrip, roundtrip)
    records.append({
        'source': str(source),
        'sha256': {p.name: hashlib.sha256(v).hexdigest() for p, v in zip((source, encoded, restored), raw)},
        'samples': len(data[0]), 'extended_samples': extended,
        'max_error': max_error, 'max_roundtrip_error': max_roundtrip,
    })
report = {
    'model': 'sign(x) * abs(x) ** (256/563); existing Adobe exponent, not literal gamma 2.2',
    'bound': '2e-5 * max(1, abs(reference)) for encoding and roundtrip',
    'presentations': len(records),
    'extended_samples': sum(r['extended_samples'] for r in records),
    'max_error': max(r['max_error'] for r in records),
    'max_roundtrip_error': max(r['max_roundtrip_error'] for r in records),
    'records': records,
}
args.report.parent.mkdir(parents=True, exist_ok=True)
args.report.write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps({k: v for k, v in report.items() if k != 'records'}))
