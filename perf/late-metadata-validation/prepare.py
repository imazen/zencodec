#!/usr/bin/env python3
"""Freeze the application and apply review-only late-description controls."""
import argparse
from pathlib import Path
import shutil
import subprocess

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('repository', type=Path)
p.add_argument('revision')
p.add_argument('output', type=Path)
a = p.parse_args()
a.output.mkdir(parents=True, exist_ok=False)
base = a.output / 'before'
base.mkdir()
archive = subprocess.Popen(['git', '-C', str(a.repository), 'archive', a.revision], stdout=subprocess.PIPE)
subprocess.run(['tar', '-x', '-C', str(base)], stdin=archive.stdout, check=True, timeout=60)
archive.stdout.close()
if archive.wait(timeout=10):
    raise SystemExit('source archive failed')
patches = Path(__file__).resolve().parent
for name in ['consumer-test.patch', 'orientation-control.patch']:
    with (patches / name).open() as patch:
        subprocess.run(['patch', '--batch', '--fuzz=0', '-p1'], cwd=base, stdin=patch, check=True, timeout=10)
shutil.copytree(base, a.output / 'after')
with (patches / 'candidate.patch').open() as patch:
    subprocess.run(['patch', '--batch', '--fuzz=0', '-p1'], cwd=a.output / 'after', stdin=patch, check=True, timeout=10)
(a.output / 'revision.txt').write_text(subprocess.check_output(
    ['git', '-C', str(a.repository), 'rev-parse', a.revision], text=True, timeout=10))
