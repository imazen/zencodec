#!/usr/bin/env python3
"""Qualify a committed spike in an isolated export, preserving failed receipts."""
import argparse
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path, help='new receipt/export directory')
    parser.add_argument('--revision', required=True)
    parser.add_argument('--target', type=Path, required=True)
    parser.add_argument('--phase', choices=['core', 'multipass', 'allocations'], default='core')
    parser.add_argument('--baseline', help='required baseline revision for allocation comparison')
    args = parser.parse_args()
    if args.phase == 'allocations' and not args.baseline:
        parser.error('--baseline is required for allocation comparison')
    root = Path(__file__).resolve().parents[1]
    revision = subprocess.check_output(['git', 'rev-parse', args.revision], cwd=root, text=True).strip()
    args.output.mkdir(parents=True, exist_ok=False)
    source = args.output.resolve() / 'source'
    source.mkdir()
    archive = subprocess.check_output(['git', 'archive', revision], cwd=root)
    with tarfile.open(fileobj=io.BytesIO(archive)) as files:
        files.extractall(source, filter='data')
    env = dict(os.environ, CARGO_TARGET_DIR=str(args.target.resolve()))
    receipt = {'revision': revision, 'phase': args.phase, 'status': 'running', 'steps': []}
    record = args.output / 'receipt.json'

    def save():
        record.write_text(json.dumps(receipt, indent=2) + '\n')

    def run(label, command):
        entry = {'label': label, 'command': command, 'status': 'running'}
        receipt['steps'].append(entry)
        receipt['stage'] = label
        save()
        with (args.output / (label + '.log')).open('wb') as log:
            try:
                rc = subprocess.run(command, cwd=source, env=env, stdout=log,
                                    stderr=subprocess.STDOUT, timeout=900).returncode
            except subprocess.TimeoutExpired:
                rc = 124
        entry.update(returncode=rc, status='passed' if rc == 0 else 'failed')
        if rc:
            receipt['status'] = 'failed'
        save()
        print(label, rc, flush=True)
        if rc:
            raise SystemExit(rc)

    if args.phase == 'core':
        run('metadata', ['cargo', 'metadata', '--format-version', '1', '--all-features'])
        run('tests', ['cargo', 'test', '--workspace', '--all-features', '--lib', '--tests', '--locked'])
        run('doctests', ['cargo', 'test', '--workspace', '--all-features', '--doc', '--locked'])
        run('lint', ['cargo', 'clippy', '--workspace', '--all-features', '--all-targets', '--locked', '--', '-D', 'warnings'])
        run('no-default', ['cargo', 'check', '-p', 'zencodec', '--no-default-features', '--locked'])
    elif args.phase == 'multipass':
        directory = 'perf/multipass-validation/native'
        native = source / directory
        (native / 'Cargo.lock').write_bytes((native / 'dependencies.lock').read_bytes())
        manifest = directory + '/Cargo.toml'
        run('native-metadata', ['cargo', 'metadata', '--manifest-path', manifest, '--format-version', '1'])
        run('native-tests', ['cargo', 'test', '--manifest-path', manifest, '--locked', '--test', 'public_driver', '--', '--test-threads=1'])
        run('native-lint', ['cargo', 'clippy', '--manifest-path', manifest, '--locked', '--test', 'public_driver', '--', '-D', 'warnings'])
    else:
        import hashlib
        directory = Path('perf/jpeg-policy-validation')
        native = source / directory
        (native / 'Cargo.lock').write_bytes((native / 'dependencies.lock').read_bytes())
        manifest = str(directory / 'Cargo.toml')
        run('allocation-metadata', ['cargo', 'metadata', '--manifest-path', manifest, '--format-version', '1'])
        baseline = subprocess.check_output(['git', 'rev-parse', args.baseline], cwd=root, text=True).strip()
        baseline_source = args.output.resolve() / 'baseline'
        baseline_source.mkdir()
        with tarfile.open(fileobj=io.BytesIO(subprocess.check_output(['git', 'archive', baseline], cwd=root))) as files:
            files.extractall(baseline_source, filter='data')
        # Exact same native harness, oracle assertions and dependency graph in
        # both arms. Only core/IO runtime comes from the respective revision.
        for file in ['Cargo.toml', 'Cargo.lock', 'src/lib.rs', 'src/row_spike.rs']:
            (baseline_source / directory / file).write_bytes((native / file).read_bytes())
        receipt['baseline'] = baseline
        receipt['harness_sha256'] = hashlib.sha256((native / 'src/row_spike.rs').read_bytes()).hexdigest()
        sources = [('baseline', baseline_source), ('candidate', source)]
        for arm, tree in sources:
            label = arm + '-build'
            command = ['cargo', 'test', '--manifest-path', str(tree / directory / 'Cargo.toml'),
                       '--locked', '--lib', '--no-run', '--message-format=json']
            run(label, command)
            artifacts = []
            for line in (args.output / (label + '.log')).read_text().splitlines():
                try:
                    message = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if message.get('reason') == 'compiler-artifact' and message.get('executable'):
                    artifacts.append(message['executable'])
            if len(artifacts) != 1:
                raise RuntimeError(f'expected one native test binary, got {artifacts}')
            binary = artifacts[0]
            receipt[arm + '_binary_sha256'] = hashlib.sha256(Path(binary).read_bytes()).hexdigest()
            for repeat in range(2):
                profile = args.output.resolve() / f'{arm}-{repeat}.heaptrack'
                run(f'{arm}-{repeat}-profile', ['heaptrack', '-o', str(profile), binary,
                    '--exact', 'row_spike::native_shared_engine_raw_typed_erased_preserve_padded_u8_u16_rgb_gray',
                    '--test-threads=1'])
                profiles = sorted(args.output.resolve().glob(profile.name + '*'))
                if len(profiles) != 1:
                    raise RuntimeError(f'expected one heaptrack capture, got {profiles}')
                run(f'{arm}-{repeat}-summary', ['heaptrack_print', '-f', str(profiles[0]), '-p', '0', '-a', '0', '-T', '0'])
        save()
    receipt['status'] = 'passed'
    save()


if __name__ == '__main__':
    main()
