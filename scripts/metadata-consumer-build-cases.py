#!/usr/bin/env python3
"""Archive exact consumer revisions and emit cases for measure-metadata-builds.py.
Requires sibling git repos in --zen-root. No checkout/worktree changes.
"""
import argparse, io, json, pathlib, subprocess, tarfile, tomllib
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--zen-root', type=pathlib.Path, required=True)
p.add_argument('--work', type=pathlib.Path, required=True)
a = p.parse_args(); a.work.mkdir(parents=True, exist_ok=True)
# Pushed, immutable implementation checkpoints. Follow-up CI-only commits do not
# change the evaluated retention implementation. Update explicitly for a new run.
revisions = {
    'ultrahdr': ('633f71e9', 'd77976b4eb0a8c12909722a12183ca3e586b15a5'),
    'heic': ('c45113e4', 'bdee7181eff59d2a42ee76e81b0e7123347dcedb'),
    'zenpipe': ('6a5b052b', '2b5ad681609288ac844e231745e4efa98a9530ea'),
}
def patches(manifest, replacements, original):
    patch = tomllib.loads(manifest.read_text()).get('patch', {})
    patch.setdefault('crates-io', {}).update(replacements)
    out = ''
    for registry, entries in patch.items():
        out += '\n[patch.' + json.dumps(registry) + ']\n'
        for name, value in entries.items():
            if 'path' in value:
                path = (manifest.parent / value['path']).resolve()
                if not path.exists(): path = (original / value['path']).resolve()
                value['path'] = str(path)
            def encode(v):
                return str(v).lower() if isinstance(v, bool) else json.dumps(v)
            out += name + ' = {' + ', '.join(k+' = '+encode(v) for k,v in value.items()) + '}\n'
    return out
cases = {}
for repo, pair in revisions.items():
    for label, rev in zip(('before','after'), pair):
        directory = a.work / f'{repo}-{label}'; directory.mkdir(exist_ok=True)
        archive = subprocess.check_output(['git','archive',rev], cwd=a.zen_root/repo)
        with tarfile.open(fileobj=io.BytesIO(archive)) as f: f.extractall(directory, filter='data')
        if repo == 'ultrahdr':
            package, path, features = 'ultrahdr-core', directory/'ultrahdr-core', []
        elif repo == 'heic':
            package, path, features = 'heic', directory, ['backend-rust','std','zencodec']
        else:
            package, path, features = 'zencodecs', directory/'zencodecs', ['jpeg-ultrahdr']
        replacements = {'zencodec': {'git':'https://github.com/imazen/zencodec', 'rev': 'cfed5cbb21ea63a5d610d1a58a6fe1f8ae8f1975' if label == 'before' else '06b5954cbd9d15282509011aab8f8efa0d8973dd'}}
        # Freeze otherwise-floating consumer graph edges for the comparison.
        if repo == 'zenpipe' and label == 'before':
            replacements['zenjpeg'] = {'git':'https://github.com/imazen/zenjpeg','rev':'fd36fd472ca88526eccf8a78788026e704042ba6'}
            replacements['heic'] = {'git':'https://github.com/imazen/heic','rev':'c45113e442c485bb9b4ea37ad53e4d8e82132205'}
            replacements['ultrahdr-core'] = {'git':'https://github.com/imazen/ultrahdr','rev':'633f71e9d0c8fc82cc29b58ffaa92d367d82f9f9'}
        cases[f'{package}-{label}'] = f'{package} = {{path={json.dumps(str(path))}, default-features=false, features={json.dumps(features)}}}\n' + patches(directory/'Cargo.toml', replacements, a.zen_root/repo)
(a.work/'cases.json').write_text(json.dumps(cases,indent=2)+'\n')
print(a.work/'cases.json')
