#!/usr/bin/env python3
"""Export a pinned native decoder and bridge it into an isolated core export.

Run through run-heavy: Cargo resolution is explicit and its lock is retained.
No original checkout or registry sources are modified.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tomllib

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('core_repository', type=Path)
p.add_argument('core_revision')
p.add_argument('jpeg_repository', type=Path)
p.add_argument('output', type=Path)
a = p.parse_args()
out = a.output.resolve()
out.mkdir(parents=True, exist_ok=False)
native_rev = '7c5a9501270210d6ef534c23221ff2660508b7cf'

def export(repo, revision, target):
    target.mkdir()
    archive = subprocess.Popen(['git', '-C', str(repo), 'archive', revision], stdout=subprocess.PIPE)
    subprocess.run(['tar', '-x', '-C', str(target)], stdin=archive.stdout, check=True, timeout=60)
    archive.stdout.close()
    if archive.wait(timeout=10):
        raise SystemExit('source archive failed')

export(a.core_repository, a.core_revision, out / 'core')
export(a.jpeg_repository, native_rev, out / 'native')
patch = out / 'core/perf/retained-decode-validation/jpeg.patch'
with patch.open() as source:
    subprocess.run(['patch', '--batch', '--fuzz=0', '-p1'], cwd=out / 'native', stdin=source, check=True, timeout=10)
harness = out / 'core/perf/jpeg-policy-validation'
manifest = harness / 'Cargo.toml'
original = manifest.read_text()
old = f'zenjpeg = {{ git = "https://github.com/imazen/zenjpeg", rev = "{native_rev}" }}'
assert original.count(old) == 1
path = json.dumps(str(out / 'native/zenjpeg'))
manifest.write_text(original.replace(old, f'zenjpeg = {{ path = {path} }}') +
                    f'\n[patch."https://github.com/imazen/zenjpeg"]\nzenjpeg = {{ path = {path} }}\n')
shutil.copyfile(harness / 'dependencies.lock', harness / 'Cargo.lock')
with (out / 'graph.json').open('w') as graph:
    subprocess.run(['cargo', 'metadata', '--manifest-path', str(manifest), '--format-version', '1'],
                   cwd=out, stdout=graph, check=True, timeout=180)
baseline = tomllib.loads((harness / 'dependencies.lock').read_text())['package']
resolved = tomllib.loads((harness / 'Cargo.lock').read_text())['package']
def identities(packages):
    return {(v['name'], v['version'], v.get('source')) for v in packages}
old, new = identities(baseline), identities(resolved)
expected = {('zenjpeg', '0.9.0'), ('zenyuv', '0.1.3')}
removed, added = old - new, new - old
if ({(n, v) for n, v, _ in removed} != expected or
        {(n, v) for n, v, _ in added} != expected or
        any(source is not None for _, _, source in added) or
        any(source != f'git+https://github.com/imazen/zenjpeg?rev={native_rev}#{native_rev}'
            for _, _, source in removed)):
    raise SystemExit(f'Unexpected dependency churn: removed={removed}, added={added}')
(out / 'lock-audit.json').write_text(json.dumps(dict(removed=sorted(removed), added=sorted(added)), indent=2) + '\n')
(out / 'identity.json').write_text(json.dumps(dict(
    core_revision=subprocess.check_output(['git', '-C', str(a.core_repository), 'rev-parse', a.core_revision], text=True).strip(),
    native_revision=native_rev,
    patch_sha256=hashlib.sha256(patch.read_bytes()).hexdigest(),
    scope='native experiment; review graph.json and Cargo.lock against dependencies.lock before tests',
), indent=2) + '\n')
