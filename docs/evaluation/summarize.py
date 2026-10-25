#!/usr/bin/env python3
"""Produce compact checked-in measurements from preserved assessment artifacts."""
import argparse
import csv
import json
from pathlib import Path
import re

p = argparse.ArgumentParser()
p.add_argument('artifacts', type=Path)
p.add_argument('output', type=Path)
p.add_argument('cohorts', nargs='+', choices=['baseline', 'revised'])
a = p.parse_args()
rows = []
for cohort in a.cohorts:
    for task in ['t1', 't2', 't3', 't4']:
        for stage in ['visible', 'expanded']:
            name = f'{task}-{stage}'
            packet = json.loads((a.artifacts / cohort / f'{name}.json').read_text())
            result = json.loads((a.artifacts / f'{cohort}-results' / name / 'result.json').read_text())
            notes = (a.artifacts / f'{cohort}-answers' / f'{name}.md').read_text()
            confidence = re.search(r'Confidence:\s*(\d+)\s*/\s*100', notes)
            assert confidence, f'Missing confidence: {cohort}/{name}'
            rows.append([cohort, task, stage, packet['revision'], packet['words'],
                         packet['rendered_height'], result['returncode'], confidence[1]])
with a.output.open('w') as f:
    writer = csv.writer(f)
    writer.writerow(['cohort', 'task', 'stage', 'revision', 'packet_words',
                     'section_css_px', 'compile_test_returncode', 'self_reported_confidence'])
    writer.writerows(rows)
print(f'Wrote {len(rows)} attempts to {a.output}')
