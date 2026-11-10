#!/usr/bin/env python3
"""Test a committed core stack in an isolated export and preserve every receipt."""
import argparse
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path, help='fresh run directory')
    parser.add_argument('--revision', default='HEAD')
    parser.add_argument('--target', type=Path, help='owned reusable Cargo target directory')
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    revision = subprocess.check_output(['git', '-C', str(repo), 'rev-parse', args.revision], text=True).strip()
    args.output.mkdir(parents=True, exist_ok=False)
    source = args.output.resolve() / 'source'
    source.mkdir()
    archive = subprocess.check_output(['git', '-C', str(repo), 'archive', revision])
    with tarfile.open(fileobj=io.BytesIO(archive)) as files:
        files.extractall(source, filter='data')
    env = dict(os.environ)
    if args.target:
        env['CARGO_TARGET_DIR'] = str(args.target.resolve())
    receipt = {'revision': revision, 'status': 'running', 'steps': []}
    record = args.output / 'receipt.json'

    def save():
        record.write_text(json.dumps(receipt, indent=2) + '\n')

    def run(label, command):
        entry = {'label': label, 'command': command, 'status': 'running'}
        receipt['steps'].append(entry)
        receipt['stage'] = label
        save()
        with (args.output / (label + '.log')).open('wb') as log:
            result = subprocess.run(command, cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT)
        entry['returncode'] = result.returncode
        entry['status'] = 'passed' if result.returncode == 0 else 'failed'
        if result.returncode:
            receipt['status'] = 'failed'
            save()
            raise SystemExit(result.returncode)
        save()

    for manifest, name in [('Cargo.toml', 'core'), ('zencodec-render/Cargo.toml', 'render')]:
        lock = source / Path(manifest).parent / 'Cargo.lock'
        seed = source / Path(manifest).parent / 'dependencies.lock'
        if seed.exists():
            lock.write_bytes(seed.read_bytes())
        run(name + '-metadata', ['cargo', 'metadata', '--manifest-path', manifest, '--format-version', '1', '--all-features'])
        if name == 'core':
            run('core-tests', ['cargo', 'test', '--workspace', '--all-features', '--lib', '--tests', '--locked'])
            run('core-docs', ['cargo', 'test', '--workspace', '--all-features', '--doc', '--locked'])
            run('core-clippy', ['cargo', 'clippy', '--workspace', '--all-features', '--all-targets', '--locked', '--', '-D', 'warnings'])
            run('core-no-default', ['cargo', 'check', '-p', 'zencodec', '--no-default-features', '--locked'])
        else:
            run('render-tests', ['cargo', 'test', '--manifest-path', manifest, '--locked'])
            run('render-clippy', ['cargo', 'clippy', '--manifest-path', manifest, '--all-targets', '--locked', '--', '-D', 'warnings'])
    receipt['status'] = 'passed'
    save()
    print(record)


if __name__ == '__main__':
    main()
