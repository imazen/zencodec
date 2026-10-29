#!/usr/bin/env python3
"""List lock changes and reject unexplained packages before native tests."""
import argparse
import json
from pathlib import Path
import tomllib

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("root", type=Path)
parser.add_argument("--portable", action="store_true",
                    help="require one complete shared family, without a legacy core copy")
args = parser.parse_args()
root = args.root
before = tomllib.loads((root / 'Cargo.lock.before').read_text())
after = tomllib.loads((root / 'Cargo.lock').read_text())
allowed = {'zencodec', 'zencodec-io', 'zencodec-coroutine', 'zencodec-testkit',
           'zencodec-media', 'zencodec-media-testkit', 'zenmeta', 'zenpixels',
           'zenpixels-convert'}
def grouped(lock):
    result = {}
    for item in lock.get('package', []):
        result.setdefault(item['name'], []).append(item)
    return result
b, a = grouped(before), grouped(after)
changes = []
unreviewed = []
for name in sorted(b.keys() | a.keys()):
    old, new = b.get(name, []), a.get(name, [])
    if old == new:
        continue
    # A source-qualified dependency ID can change on a package whose own
    # identity and other dependencies are untouched.
    strip = lambda item: {k: v for k, v in item.items() if k != 'dependencies'}
    external = lambda items: sorted(d for i in items for d in i.get('dependencies', [])
                                    if d.split()[0] not in allowed)
    explained = name in allowed or ([strip(i) for i in old] == [strip(i) for i in new]
                                    and external(old) == external(new))
    changes.append({'name': name, 'before': old, 'after': new, 'explained_family_bridge': explained})
    if not explained:
        unreviewed.append(name)
core = [i for i in after.get('package', []) if i['name'] == 'zencodec' and i['version'].startswith('0.2.')]
graph_errors = []
if args.portable:
    for name in ('zencodec', 'zenpixels', 'how-far'):
        if len(a.get(name, [])) != 1:
            graph_errors.append(f'{name}: expected exactly one package')
    for name in allowed:
        if len(a.get(name, [])) > 1:
            graph_errors.append(f'{name}: duplicate family packages')
    if len(core) == 1:
        for name in sorted(allowed - {'zenpixels', 'zenpixels-convert'}):
            for item in a.get(name, []):
                if item.get('source') != core[0].get('source'):
                    graph_errors.append(f'{name}: source differs from core')
    if len(a.get('zenpixels', [])) == 1:
        for item in a.get('zenpixels-convert', []):
            if item.get('source') != a['zenpixels'][0].get('source'):
                graph_errors.append('zenpixels-convert: source differs from pixels')
result = {'changes': changes, 'unreviewed': unreviewed, 'shared_core': core,
          'graph_errors': graph_errors,
          'scope': ('portable graph: unique shared family and paired pixel source'
                    if args.portable else
                    'isolated experiment: 0.1 native-only test dependencies may coexist; shared 0.2 must be unique')}
(root / 'OPTIONAL-LOCK-AUDIT.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps({'changed': len(changes), 'unreviewed': unreviewed, 'shared_core_count': len(core), 'graph_errors': graph_errors}))
if unreviewed or len(core) != 1 or graph_errors:
    raise SystemExit(1)
