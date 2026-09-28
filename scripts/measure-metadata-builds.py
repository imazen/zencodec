#!/usr/bin/env python3
"""Fresh-cache compile comparison. Artifacts/logs stay in --work for inspection.
Run without concurrent builds; cargo fetch is excluded from timing. No worktrees.
"""
import argparse, io, json, os, pathlib, platform, statistics, subprocess, tarfile, time, shutil
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--work', type=pathlib.Path, required=True)
p.add_argument('--baseline', default='cfed5cb')
p.add_argument('--repeats', type=int, default=3)
p.add_argument('--extra-cases', type=pathlib.Path, help='JSON map of additional case names to dependency/patch TOML')
p.add_argument('--only', nargs='*', help='Only run selected cases')
a = p.parse_args()
root = pathlib.Path(__file__).resolve().parents[1]
a.work.mkdir(parents=True, exist_ok=True)
baseline = a.work / 'baseline'
baseline.mkdir(exist_ok=True)
archive = subprocess.check_output(['git', 'archive', a.baseline], cwd=root)
with tarfile.open(fileobj=io.BytesIO(archive)) as f: f.extractall(baseline, filter='data')
snapshot = a.work / 'current'
snapshot.mkdir(exist_ok=True)
files = subprocess.check_output(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'], cwd=root).decode().split('\0')
for name in files:
    if name and (root/name).is_file():
        (snapshot/name).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(root/name, snapshot/name)
cases = {
    'zencodec-baseline': f'zencodec = {{ path = {json.dumps(str(baseline))}, default-features = false }}',
    'zencodec-default': f'zencodec = {{ path = {json.dumps(str(snapshot))}, default-features = false }}',
    'zencodec-audit': f'zencodec = {{ path = {json.dumps(str(snapshot))}, default-features = false, features = ["metadata-audit"] }}',
    'zencodec-xmp': f'zencodec = {{ path = {json.dumps(str(snapshot))}, default-features = false, features = ["xmp"] }}',
    'roxmltree': 'roxmltree = { version = "=0.21.1", default-features = false, features = ["positions"] }',
    'kamadak-exif': 'kamadak-exif = "=0.6.1"',
    'xmpkit-core': 'xmpkit = { version = "=0.1.6", default-features = false, features = ["core"] }',
    'xmpkit-default': 'xmpkit = "=0.1.6"',
}
if a.extra_cases: cases.update(json.loads(a.extra_cases.read_text()))
if a.only: cases = {name: dep for name, dep in cases.items() if name in a.only}
report = {'rustc': subprocess.check_output(['rustc','-Vv'], text=True), 'platform': platform.platform(), 'baseline': a.baseline, 'jobs': 4, 'cpu': pathlib.Path('/proc/cpuinfo').read_text().split('model name')[1].split('\n')[0].strip(), 'cases': {}}
for name, dep in cases.items():
    project = a.work / name
    (project/'src').mkdir(parents=True, exist_ok=True)
    (project/'Cargo.toml').write_text(f'[package]\nname="metadata-build-probe"\nversion="0.0.0"\nedition="2024"\n[dependencies]\n{dep}\n')
    source = project/'src/lib.rs'
    source.write_text('pub fn probe() -> usize { 1 }\n')
    with (project/'fetch.log').open('w') as log:
        subprocess.run(['cargo','fetch'], cwd=project, stdout=log, stderr=log, check=True)
    report.setdefault('dependencies', {})[name] = subprocess.check_output(['cargo', 'tree', '--edges', 'normal', '--prefix', 'none', '--locked'], cwd=project, text=True)
    results = []
    for repeat in range(a.repeats):
        env = dict(os.environ, CARGO_TARGET_DIR=str(project/f'target-{repeat}'))
        times = {}
        for stage in ['cold', 'warm', 'edited']:
            if stage == 'edited': source.write_text(source.read_text()+'// source invalidation\n')
            start = time.perf_counter()
            with (project/f'{repeat}-{stage}.log').open('w') as log:
                result = subprocess.run(['cargo','build','--lib','--offline','--locked','-j4'], cwd=project, env=env, stdout=log, stderr=log)
            times[stage] = {'seconds': time.perf_counter()-start, 'exit': result.returncode}
            if result.returncode: break
        results.append(times)
        if result.returncode: break
    report['cases'][name] = results
    (a.work/'results.json').write_text(json.dumps(report, indent=2)+'\n')
    print(name, json.dumps(results), flush=True)
