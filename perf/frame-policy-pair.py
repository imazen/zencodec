#!/usr/bin/env python3
"""Compare retained fixed-work policy probes with zenbench pair.

Wall metrics include Python/process startup and correctness assertions. The
probe verifies the actual bounded report before this wrapper reports success.
No pixel decoding or conversion is measured.
"""
import argparse
import json
from pathlib import Path
import subprocess


def prepare(root, before, after):
    script = Path(__file__).resolve()
    plan = {
        "schema": 1,
        "cases": [{"id": mode, "inputs": {"mode": str(root / (mode + '.json')),
                   "runner": str(script), "before_probe": str(before), "after_probe": str(after)}}
                  for mode in ['record', 'resolve', 'combined']],
        "metric": {"extract": {"kind": "wall"}, "unit": "ns", "expected": 1,
                   "divide_by": 1, "resolution": 1000000},
        "check": {"kind": "json", "prefix": "result=", "pointers": ["/verified"]},
        "warmup": "Saved release probes; 10000000 operations per process, including startup and final report assertions; OS cache warm; no decode or conversion.",
        "blocks": 5, "aa": True, "deadline_seconds": 300,
    }
    for mode in ['record', 'resolve', 'combined']:
        path = root / (mode + '.json')
        with path.open('x') as file:
            json.dump({"mode": mode, "count": 10000000}, file)
    arms = [{"id": name, "program": "/usr/bin/python3", "revision": name,
             "args": [str(script), 'run', str(probe), '${mode}'], "timeout_seconds": 60}
            for name, probe in [('before', before), ('after', after)]]
    plan['baseline'], plan['candidates'] = arms[0], arms[1:]
    with (root / 'pair-plan.json').open('x') as file:
        json.dump(plan, file, indent=2)
        file.write('\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action', required=True)
    p = sub.add_parser('prepare')
    p.add_argument('root', type=Path)
    p.add_argument('before', type=Path)
    p.add_argument('after', type=Path)
    p = sub.add_parser('run')
    p.add_argument('probe', type=Path)
    p.add_argument('mode', type=Path)
    args = parser.parse_args()
    if args.action == 'prepare':
        prepare(args.root.resolve(), args.before.resolve(), args.after.resolve())
    else:
        mode = json.loads(args.mode.read_text())
        subprocess.run([str(args.probe), mode['mode'], str(mode['count'])], check=True, timeout=60)
        print('result={"verified":true}')
