#!/usr/bin/env python3
"""Require successful full decodes, then compare each stream's ordered frames.

The pinned packet/seek oracles separately check timestamps and keyframe flags.
This check ignores timestamps so legal container retiming does not hide pixel
or audio-sample differences. It never accepts two matching partial decodes.
"""
import argparse
from collections import defaultdict
from pathlib import Path
import subprocess

p = argparse.ArgumentParser()
p.add_argument("mp4", type=Path)
p.add_argument("webm", type=Path)
p.add_argument("out", type=Path)
args = p.parse_args()
args.out.mkdir(parents=True, exist_ok=False)
cache = {}


def decode(path):
    if path in cache:
        return cache[path]
    run = subprocess.run([
        "ffmpeg", "-v", "error", "-xerror", "-err_detect", "explode",
        "-i", str(path), "-map", "0", "-f", "framemd5", "-",
    ], capture_output=True, check=False)
    (args.out / (path.name + ".md5")).write_bytes(run.stdout)
    (args.out / (path.name + ".stderr")).write_bytes(run.stderr)
    if run.returncode:
        raise RuntimeError(f"{path.name}: decode exit {run.returncode}: {run.stderr.decode()}")
    streams = defaultdict(list)
    for line in run.stdout.decode().splitlines():
        if not line or line.startswith("#"):
            continue
        fields = [v.strip() for v in line.split(",")]
        assert len(fields) == 6, line
        streams[int(fields[0])].append((int(fields[4]), fields[5]))
    assert streams and all(streams.values()), f"no decoded frames: {path}"
    print(f"{path.name}: exit=0 frames={sum(map(len, streams.values()))} streams={len(streams)} diagnostics={bool(run.stderr)}")
    cache[path] = dict(streams)
    return cache[path]


pairs = []
for copy in sorted(args.mp4.glob("*.zen-*.mp4")):
    pairs.append((copy.with_name(copy.name.split(".zen-")[0] + ".mp4"), copy))
for copy in sorted(args.webm.glob("*.zen.*")):
    if copy.suffix in (".webm", ".mkv"):
        pairs.append((copy.with_name(copy.name.replace(".zen.", ".")), copy))
assert len(pairs) == 23, f"expected all 23 copies from the pinned oracles, got {len(pairs)}"
for original, copy in pairs:
    assert decode(original) == decode(copy), f"decoded content differs: {original} -> {copy}"
print(f"strict decoded-content parity: {len(pairs)}/{len(pairs)} copies; {len(cache)} complete decodes")
