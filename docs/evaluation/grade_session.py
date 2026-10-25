#!/usr/bin/env python3
"""Grade both frozen attempts after a sequential reader finishes all tasks."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

p = argparse.ArgumentParser()
p.add_argument('reader', type=Path)
p.add_argument('export', type=Path)
p.add_argument('output', type=Path)
a = p.parse_args()
assert (a.reader / 't6-expanded.md').exists(), 'Finish the intake before grading'
records = []
for task in ['t1', 't2', 't3', 't4']:
    for stage in ['visible', 'expanded']:
        name = f'{task}-{stage}'
        output = a.output / name
        assert not output.exists(), f'Preserve existing results: {output}'
        subprocess.run([
            sys.executable, str(Path(__file__).with_name('run.py')), task,
            str(a.reader / f'{name}.rs'), str(a.export), str(output),
        ], check=True)
        record = json.loads((output / 'result.json').read_text())
        records.append({'task': task, 'stage': stage, 'returncode': record['returncode']})
(a.output / 'summary.json').write_text(json.dumps(records, indent=2) + '\n')
for stage in ['visible', 'expanded']:
    passed = sum(r['returncode'] == 0 for r in records if r['stage'] == stage)
    print(f'{stage}: {passed}/4 compile-and-run oracles passed')
