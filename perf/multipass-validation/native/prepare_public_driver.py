#!/usr/bin/env python3
"""Prepare an immutable native/public-driver export. Never invokes Cargo."""
import argparse
import difflib
import hashlib
import json
from pathlib import Path
import shlex
import shutil
import subprocess
import tarfile
import tomllib

CORE = 'd7b76287337a1d99ffa7c202814e8f87e8044369'
NATIVE = 'd3e0eac5cdcc557513cee043efdb986e46a6e307'
PIXELS = '283655c5424e0ad7ebc6d2acd94670fac1e29158'
HOW_FAR = '571bf1a2fc92e5b99c3777f8ff81bafab7666a75'
FAMILY = {'zencodec': '.', 'zencodec-io': 'zencodec-io',
          'zencodec-testkit': 'zencodec-testkit',
          'zencodec-coroutine': 'zencodec-coroutine', 'zenmeta': 'zenmeta'}
HERE = Path(__file__).resolve().parent


def archive(git_dir, rev, destination):
    identity = subprocess.check_output(
        ['git', f'--git-dir={git_dir}', 'rev-parse', rev + '^{commit}'], text=True).strip()
    if identity != rev:
        raise SystemExit('Frozen commit identity differs: ' + identity)
    destination.mkdir()
    with subprocess.Popen(['git', f'--git-dir={git_dir}', 'archive', rev],
                          stdout=subprocess.PIPE) as proc:
        with tarfile.open(fileobj=proc.stdout, mode='r|') as tar:
            tar.extractall(destination, filter='data')
        if proc.wait() != 0:
            raise SystemExit('git archive failed')


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('destination', type=Path, help='new directory outside ambient workspace patches')
    parser.add_argument('--core-git-dir', type=Path, required=True)
    parser.add_argument('--native-git-dir', type=Path, required=True)
    args = parser.parse_args()
    root = args.destination.resolve()
    # Creating a new directory keeps every earlier export and run intact.
    root.mkdir()
    core, native, run = root / 'core', root / 'native', root / 'run'
    run.mkdir()
    archive(args.core_git_dir.resolve(), CORE, core)
    archive(args.native_git_dir.resolve(), NATIVE, native)
    harness = core / 'perf/multipass-validation/native'
    baseline = harness / 'dependencies.lock'
    shutil.copyfile(baseline, harness / 'Cargo.lock')
    shutil.copyfile(baseline, run / 'original.Cargo.lock')
    preserved = {str(p.relative_to(harness)): digest(p)
                 for p in sorted(harness.rglob('*.rs'))}
    # Overlay only this lane's new controls and source-owned qualification tools.
    for relative in ['tests/public_driver.rs', 'tests/public_driver/support.rs',
                     'audit_public_driver.py', 'prepare_public_driver.py', 'README.md']:
        src, dst = HERE / relative, harness / relative
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src, dst)
    for relative, sha in preserved.items():
        if digest(harness / relative) != sha:
            raise SystemExit('Existing native harness source changed: ' + relative)
    manifest = harness / 'Cargo.toml'
    before = manifest.read_text()
    lines = before.splitlines(keepends=True)
    replaced = 0
    for i, line in enumerate(lines):
        if line.startswith('imij = '):
            lines[i] = ('imij = { path = ' + json.dumps(str(native / 'imij'))
                        + ', default-features = false, features = ["std", "encode"] }\n')
            replaced += 1
    if replaced != 1:
        raise SystemExit('Expected one native dependency to overlay')
    text = ''.join(lines)
    # Registry edges in imij are the only indirect core-family edges active in
    # this consumer graph; no speculative git patch is added for native dev deps.
    text = text.replace('zencodec = { path = "../../.." }',
                        'zencodec = { path = ' + json.dumps(str(core)) + ' }')
    parsed = tomllib.loads(text)
    patch = parsed['patch']['crates-io']
    for name, relative in FAMILY.items():
        if name not in patch:
            text += name + ' = { path = ' + json.dumps(str((core / relative).resolve())) + ' }\n'
    parsed = tomllib.loads(text)
    for name in ['zenpixels', 'zenpixels-convert']:
        assert parsed['patch']['crates-io'][name]['rev'] == PIXELS
    assert parsed['patch']['crates-io']['how-far']['rev'] == HOW_FAR
    manifest.write_text(text)
    (run / 'MANIFEST.patch').write_text(''.join(difflib.unified_diff(
        before.splitlines(True), text.splitlines(True),
        fromfile='committed/native/Cargo.toml', tofile='export/native/Cargo.toml')))
    for name in ['tmp', 'target', 'scratch']:
        (run / name).mkdir()
    evidence = {
        'status': 'SOURCE PREPARED; no Cargo resolution/build/test executed',
        'core_commit': CORE, 'native_commit': NATIVE, 'pixels_commit': PIXELS,
        'how_far_commit': HOW_FAR, 'core': str(core), 'native': str(native),
        'harness': str(harness), 'run': str(run), 'core_family': FAMILY,
        'baseline_lock_sha256': digest(baseline), 'preserved_native_source': preserved,
        'overlay_source': {p: digest(harness / p) for p in [
            'tests/public_driver.rs', 'tests/public_driver/support.rs',
            'audit_public_driver.py', 'prepare_public_driver.py', 'README.md']},
        'manifest_sha256': digest(manifest),
        'frozen_source': {str(p.relative_to(root)): digest(p)
                          for directory in [core / 'src', core / 'zencodec-io/src', native / 'imij/src']
                          for p in sorted(directory.rglob('*.rs'))},
    }
    (run / 'PREPARED.json').write_text(json.dumps(evidence, indent=2) + '\n')
    q = shlex.quote
    script = f'''#!/usr/bin/env bash
set -euo pipefail
export TMPDIR={q(str(run / 'tmp'))}
export CARGO_TARGET_DIR={q(str(run / 'target'))}
export MULTIPASS_SCRATCH={q(str(run / 'scratch'))}
export CARGO_NET_GIT_FETCH_WITH_CLI=true
export CARGO_BUILD_JOBS=2
export RAYON_NUM_THREADS=2
export OMP_NUM_THREADS=2
cd {q(str(harness))}
python3 audit_public_driver.py {q(str(run))} --prepared-only
# Preserve the baseline; never generate a fresh lock or run a broad update.
preserve_lock() {{ cp Cargo.lock {q(str(run / 'resolved.Cargo.lock'))}; }}
trap preserve_lock EXIT
cargo metadata --format-version 1 > {q(str(run / 'metadata.json'))}
cp Cargo.lock {q(str(run / 'resolved.Cargo.lock'))}
python3 audit_public_driver.py {q(str(run))}
cargo test --locked --all-targets -- --test-threads=1
cargo clippy --locked --all-targets -- -D warnings
'''
    (run / 'run-public-driver.sh').write_text(script)
    command = ('set -o pipefail\n'
               + q(str(Path.home() / 'work/claudehints/scripts/run-heavy'))
               + ' --mem 3G --jobs 2 -- timeout --signal=TERM --kill-after=30s 900 bash '
               + q(str(run / 'run-public-driver.sh')) + ' 2>&1 | tee '
               + q(str(run / 'full.log')) + '\n')
    (run / 'COMMAND.sh').write_text(command)
    print(json.dumps({'prepared': str(run / 'PREPARED.json'), 'command': command}, indent=2))


if __name__ == '__main__':
    main()
