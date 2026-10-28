#!/usr/bin/env python3
"""Fail closed on source drift, split core identities or unrelated lock churn."""
import argparse
import difflib
import hashlib
import json
from pathlib import Path
import tomllib


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('run', type=Path)
    parser.add_argument('--prepared-only', action='store_true')
    args = parser.parse_args()
    run = args.run.resolve()
    prepared = json.loads((run / 'PREPARED.json').read_text())
    harness = Path(prepared['harness'])
    for relative, sha in prepared['frozen_source'].items():
        if digest(run.parent / relative) != sha:
            raise SystemExit('Frozen production source drift: ' + relative)
    for relative, sha in (prepared['preserved_native_source'] | prepared['overlay_source']).items():
        if digest(harness / relative) != sha:
            raise SystemExit('Exported source drift: ' + relative)
    for path, sha in [(harness / 'Cargo.toml', prepared['manifest_sha256']),
                      (harness / 'dependencies.lock', prepared['baseline_lock_sha256']),
                      (run / 'original.Cargo.lock', prepared['baseline_lock_sha256'])]:
        if digest(path) != sha:
            raise SystemExit('Prepared manifest/baseline drift: ' + str(path))
    if args.prepared_only:
        if digest(harness / 'Cargo.lock') != prepared['baseline_lock_sha256']:
            raise SystemExit('Resolution must start from the original lock; prepare a fresh export')
        print('Prepared source and baseline identities match; Cargo has not been invoked.')
        return
    before_path, after_path = run / 'original.Cargo.lock', run / 'resolved.Cargo.lock'
    before_text, after_text = before_path.read_text(), after_path.read_text()
    before, after = map(tomllib.loads, (before_text, after_text))
    family = set(prepared['core_family']) | {'imij', 'zenpixels', 'zenpixels-convert'}

    def packages(lock):
        result = {}
        for package in lock['package']:
            result.setdefault(package['name'], []).append(package)
        return {n: sorted(ps, key=lambda p: (p['version'], p.get('source', '')))
                for n, ps in result.items()}

    old, new = packages(before), packages(after)
    changes, unreviewed = [], []
    for name in sorted(old.keys() | new.keys()):
        a, b = old.get(name, []), new.get(name, [])
        if a == b:
            continue
        reason = ('Explicit frozen core/native/pixel migration; canonical identities checked below.'
                  if name in family else None)
        if reason is None and len(a) == len(b) == 1:
            x, y = a[0].copy(), b[0].copy()
            da, db = set(x.pop('dependencies', [])), set(y.pop('dependencies', []))
            if x == y and all(d.split()[0] in family for d in da ^ db):
                reason = 'Only references to explicitly migrated packages changed.'
        if reason is None:
            unreviewed.append(name)
            reason = 'UNREVIEWED dependency churn; source review required before build.'
        changes.append({'name': name, 'before': a, 'after': b, 'reason': reason})
    a = {k: v for k, v in before.items() if k != 'package'}
    b = {k: v for k, v in after.items() if k != 'package'}
    if a != b:
        x, y = a.copy(), b.copy()
        px, py = x.pop('patch', {}), y.pop('patch', {})
        patch_only = (x == y and set(px) <= {'unused'} and set(py) <= {'unused'}
                      and all(p['name'] in family for p in px.get('unused', []) + py.get('unused', [])))
        if not patch_only:
            unreviewed.append('<lock metadata>')
        changes.append({'name': '<lock metadata>', 'before': a, 'after': b,
                        'reason': 'Explicit unused-patch bookkeeping.' if patch_only else 'UNREVIEWED'})
    receipt = {'method': 'Incremental cargo metadata from preserved baseline; no broad update',
               'before_sha256': digest(before_path), 'after_sha256': digest(after_path),
               'changes': changes, 'unreviewed': unreviewed}
    (run / 'LOCK-DELTA.json').write_text(json.dumps(receipt, indent=2) + '\n')
    (run / 'LOCK-DELTA.patch').write_text(''.join(difflib.unified_diff(
        before_text.splitlines(True), after_text.splitlines(True),
        fromfile='original.Cargo.lock', tofile='resolved.Cargo.lock')))
    metadata = json.loads((run / 'metadata.json').read_text())
    grouped = {}
    for package in metadata['packages']:
        grouped.setdefault(package['name'], []).append(package)
    problems = []
    observed = {}
    required = {'zencodec', 'zencodec-io', 'imij', 'zenpixels', 'how-far'}
    paths = {name: Path(prepared['core']) / relative / 'Cargo.toml'
             for name, relative in prepared['core_family'].items()}
    paths['imij'] = Path(prepared['native']) / 'imij/Cargo.toml'
    for name in sorted(family | {'how-far'}):
        found = grouped.get(name, [])
        observed[name] = [{'id': p['id'], 'manifest_path': p['manifest_path'],
                           'source': p['source'], 'version': p['version']} for p in found]
        if not found and name not in required:
            continue
        if len(found) != 1:
            problems.append(name + ': missing or multiple package identities')
            continue
        p = found[0]
        if name in paths:
            if p['source'] is not None or Path(p['manifest_path']).resolve() != paths[name].resolve():
                problems.append(name + ': wrong canonical frozen path')
        else:
            repo, pin = (('how-far', prepared['how_far_commit']) if name == 'how-far'
                         else ('zenpixels', prepared['pixels_commit']))
            expected = 'git+https://github.com/imazen/' + repo + '?rev=' + pin + '#' + pin
            if p['source'] != expected:
                problems.append(name + ': wrong paired git source')
    # Verify all local packages really reside in these two immutable exports.
    for p in metadata['packages']:
        if p['source'] is None:
            path = Path(p['manifest_path']).resolve()
            if not any(path.is_relative_to(Path(prepared[key])) for key in ['core', 'native']):
                problems.append(p['name'] + ': ambient local dependency outside exports')
    graph = {'packages': observed, 'problems': problems}
    (run / 'GRAPH.json').write_text(json.dumps(graph, indent=2) + '\n')
    print(json.dumps({'lock_changes': changes, 'unreviewed': unreviewed, 'graph': graph}, indent=2))
    if unreviewed or problems:
        raise SystemExit('Refusing build: review LOCK-DELTA.json and GRAPH.json')


if __name__ == '__main__':
    main()
