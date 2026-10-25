#!/usr/bin/env python3
"""Check the frozen published baseline against checksum-verified crates.io archives.

Network is used only by this explicit recipe, never by the site build. This
verifies selected source-presence claims, not semantic equivalence or semver.
"""
import hashlib
import io
import json
from pathlib import Path
import tarfile
from urllib.request import Request, urlopen

baseline = json.loads(Path(__file__).with_name('migration-baseline.json').read_text())
cache = Path.home() / 'tmp' / 'zencodec-migration-baseline'
cache.mkdir(parents=True, exist_ok=True)
for package in baseline['packages']:
    name, version = package['name'], package['version']
    filename = f'{name}-{version}.crate'
    archive = cache / filename
    if not archive.exists():
        request = Request(f'https://static.crates.io/crates/{name}/{filename}',
                          headers={'User-Agent': 'zencodec-migration-documentation'})
        archive.write_bytes(urlopen(request, timeout=60).read())
    data = archive.read_bytes()
    assert hashlib.sha256(data).hexdigest() == package['sha256'], filename
    prefix = f'{name}-{version}/'
    with tarfile.open(fileobj=io.BytesIO(data)) as files:
        vcs = json.load(files.extractfile(prefix + '.cargo_vcs_info.json'))
        assert vcs['git']['sha1'] == package['vcs_revision'], filename
        sources = '\n'.join(files.extractfile(member).read().decode()
                            for member in files.getmembers()
                            if member.isfile() and member.name.startswith(prefix + 'src/')
                            and member.name.endswith('.rs'))
    for token in package['source_checks']['present']:
        assert token in sources, (filename, 'missing', token)
    for token in package['source_checks']['absent']:
        assert token not in sources, (filename, 'unexpected', token)
    print(f'{name} {version}: archive hash, source revision and selected API presence verified')
