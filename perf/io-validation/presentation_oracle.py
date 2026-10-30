#!/usr/bin/env python3
"""Compare native MP4 edit playback with FFmpeg; retain every mismatch.

Usage: presentation_oracle.py NATIVE_FILES SOURCE_EXPECTED_RAW NEW_RESULTS
The encoded input and raw reference originate from the native AV1 fixture.
No external encoder or rewritten container is used.
"""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
from fractions import Fraction

files, expected_raw, output = map(Path, sys.argv[1:])
output.mkdir(parents=True, exist_ok=False)
expected = expected_raw.read_bytes()
assert hashlib.sha256(expected).hexdigest() == '7894797ccaac0d7c4fb904f4e4bd1f182ab09070b0e1adc8975eea57254e8c12'
frame_bytes = 17 * 13 * 3
assert len(expected) == 3 * frame_bytes
frames = [expected[i * frame_bytes:(i + 1) * frame_bytes] for i in range(3)]
cases = {
    'identity': ([0, 1, 2], [0, 40, 80], 120),
    'clip': ([1], [0], 40),
    'repeat': ([0, 0], [0, 40], 80),
    'gap': ([0, 1], [40, 80], 120),
    'dwell': ([1], [0], 80),
    'double': ([0, 1, 2], [0, 20, 40], 60),
}
versions = {}
for tool in ['ffmpeg', 'ffprobe']:
    versions[tool] = subprocess.run([tool, '-version'], capture_output=True, text=True, check=True, timeout=10).stdout
(output / 'versions.json').write_text(json.dumps(versions, indent=2))
results = []
for name, (indices, timestamps, end) in cases.items():
    for layout in ['progressive', 'fragmented']:
        label = f'{name}-{layout}'
        source = files / f'{label}.mp4'
        raw = output / f'{label}.raw'
        probe = subprocess.run(['ffprobe', '-v', 'error', '-show_frames', '-show_streams', '-of', 'json', str(source)], capture_output=True, timeout=30)
        (output / f'{label}.probe.json').write_bytes(probe.stdout)
        (output / f'{label}.probe.stderr').write_bytes(probe.stderr)
        decode = subprocess.run(['ffmpeg', '-v', 'error', '-nostdin', '-threads', '1', '-c:v', 'libdav1d', '-i', str(source), '-fps_mode', 'passthrough', '-f', 'rawvideo', '-pix_fmt', 'gbrp', str(raw)], capture_output=True, timeout=30)
        (output / f'{label}.decode.stderr').write_bytes(decode.stderr)
        record = {'case': label, 'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest(), 'probe_rc': probe.returncode, 'decode_rc': decode.returncode, 'expected_frames': indices, 'expected_pts_ms': timestamps, 'expected_end_ms': end}
        if probe.returncode == 0 and decode.returncode == 0:
            facts = json.loads(probe.stdout)
            clock = Fraction(facts['streams'][0]['time_base']) * 1000
            decoded = facts['frames']
            pts = [Fraction(f['pts']) * clock for f in decoded]
            final = max((Fraction(f['pts']) + Fraction(f.get('duration', f.get('pkt_duration', 0)))) * clock for f in decoded) if decoded else None
            actual = raw.read_bytes()
            record.update(actual_pts_ms=list(map(str, pts)), actual_end_ms=str(final), actual_bytes=len(actual), samples_match=actual == b''.join(frames[i] for i in indices), timing_match=pts == list(map(Fraction, timestamps)) and final == end)
            record['pass'] = record['samples_match'] and record['timing_match']
        else:
            record['pass'] = False
        results.append(record)
        (output / 'results.json').write_text(json.dumps(results, indent=2) + '\n')
        print(label, 'PASS' if record['pass'] else 'MISMATCH', flush=True)
raise SystemExit(0 if all(r['pass'] for r in results) else 1)
