#!/usr/bin/env python3
"""Exercise portable graph rejection without invoking Cargo."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('audit-lock.py')
PACKAGES = [
    ('zencodec', '0.2.0', 'git+https://github.com/imazen/zencodec?rev=one#one'),
    ('zencodec-io', '0.1.0', 'git+https://github.com/imazen/zencodec?rev=one#one'),
    ('zenpixels', '0.2.17', 'git+https://github.com/imazen/zenpixels?rev=two#two'),
    ('zenpixels-convert', '0.2.17', 'git+https://github.com/imazen/zenpixels?rev=two#two'),
    ('how-far', '0.1.0', 'registry+https://github.com/rust-lang/crates.io-index'),
]


class PortableGraph(unittest.TestCase):
    def audit(self, packages, expected, diagnostic=None):
        scratch = Path.home() / 'tmp'
        scratch.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='optional-graph-', dir=scratch) as directory:
            root = Path(directory)
            lock = 'version = 4\n' + ''.join(
                '\n[[package]]\nname = ' + json.dumps(name) +
                '\nversion = ' + json.dumps(version) +
                '\nsource = ' + json.dumps(source) + '\n'
                for name, version, source in packages)
            # Equal before/after: an already-bad graph must still refuse.
            for filename in ('Cargo.lock', 'Cargo.lock.before'):
                (root / filename).write_text(lock)
            result = subprocess.run(
                ['python3', str(SCRIPT), str(root), '--portable'],
                capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
            if diagnostic:
                self.assertIn(diagnostic, result.stdout)

    def test_paired_graph(self):
        self.audit(PACKAGES, 0)

    def test_legacy_core_is_not_a_native_only_exception(self):
        self.audit(PACKAGES + [('zencodec', '0.1.26',
                   'registry+https://github.com/rust-lang/crates.io-index')],
                   1, 'zencodec: expected exactly one package')

    def test_single_core_does_not_hide_a_different_io_revision(self):
        self.audit([(n, v, 'git+https://github.com/imazen/zencodec?rev=old#old'
                     if n == 'zencodec-io' else s) for n, v, s in PACKAGES],
                   1, 'zencodec-io: source differs from core')

    def test_converter_must_share_the_pixel_source(self):
        self.audit([(n, v, 'registry+https://github.com/rust-lang/crates.io-index'
                     if n == 'zenpixels-convert' else s) for n, v, s in PACKAGES],
                   1, 'zenpixels-convert: source differs from pixels')


if __name__ == '__main__':
    unittest.main()
