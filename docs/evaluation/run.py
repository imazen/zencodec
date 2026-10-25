#!/usr/bin/env python3
"""Compile one preserved submission against the existing pinned native harness.

Run through run-heavy. No grading changes, dependency updates or source fixes.
"""
import argparse, json, pathlib, subprocess
p = argparse.ArgumentParser()
p.add_argument('task', choices=['t1','t2','t3','t4'])
p.add_argument('submission', type=pathlib.Path)
p.add_argument('export', type=pathlib.Path)
p.add_argument('output', type=pathlib.Path)
a = p.parse_args()
root = pathlib.Path(__file__).resolve().parents[2]
export = a.export.resolve()
assert not (export/'.git').exists() and not (export/'.jj').exists(), 'Use a source export'
crate = export/'perf'/('jpeg-policy-validation' if a.task=='t4' else 'io-validation')
test_name = 'guide_learning_'+a.task
source = '#![forbid(unsafe_code)]\nmod candidate {\n'+a.submission.read_text()+'\n}\n'
if a.task=='t4':
    fixture = crate/'src/fixtures.rs'
    source += '#[allow(dead_code)]\n#[path = '+json.dumps(str(fixture))+']\nmod fixtures;\n'
source += (root/'docs/evaluation/oracles'/f'{a.task}.rs').read_text()
(crate/'tests').mkdir(exist_ok=True)
test_file = crate/'tests'/f'{test_name}.rs'
assert not test_file.exists(), f'Remove the previous generated test from the export: {test_file}'
test_file.write_text(source)
a.output.mkdir(parents=True,exist_ok=True)
cmd = ['cargo','test','--manifest-path',str(crate/'Cargo.toml'),'--locked','--test',test_name,'--','--nocapture']
(a.output/'compiled_submission.rs').write_text(source)
try:
    with (a.output/'compiler.log').open('w') as log:
        result = subprocess.run(cmd,stdout=log,stderr=subprocess.STDOUT)
finally:
    test_file.unlink()
record = {'task':a.task,'submission':str(a.submission),'command':cmd,'returncode':result.returncode}
(a.output/'result.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps(record))
