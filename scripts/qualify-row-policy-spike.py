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
    parser.add_argument('--phase', choices=['core', 'multipass'], default='core')
    args = parser.parse_args()
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
    else:
        directory = 'perf/multipass-validation/native'
        native = source / directory
        (native / 'Cargo.lock').write_bytes((native / 'dependencies.lock').read_bytes())
        manifest = directory + '/Cargo.toml'
        run('native-metadata', ['cargo', 'metadata', '--manifest-path', manifest, '--format-version', '1'])
        run('native-tests', ['cargo', 'test', '--manifest-path', manifest, '--locked', '--test', 'public_driver', '--', '--test-threads=1'])
        run('native-lint', ['cargo', 'clippy', '--manifest-path', manifest, '--locked', '--test', 'public_driver', '--', '-D', 'warnings'])
    receipt['status'] = 'passed'
    save()


if __name__ == '__main__':
    main()
