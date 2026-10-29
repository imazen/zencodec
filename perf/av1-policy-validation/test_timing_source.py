"""Transport failures never weaken pinned source integrity."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import timing_source as source


class SourceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(dir=os.path.expanduser('~/tmp'))
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.payload = b'controlled source archive'
        self.sha = hashlib.sha256(self.payload).hexdigest()
        self.pin = patch.dict(source.SHA256, {'8.0.1': self.sha})
        self.pin.start()
        self.addCleanup(self.pin.stop)
        self.archive = self.root / 'ffmpeg-8.0.1.tar.xz'

    def download(self, command, **kwargs):
        self.assertEqual(kwargs, {'check': True, 'timeout': 65})
        self.assertIn('--max-time', command)
        self.assertIn('--proto-redir', command)
        Path(command[-1]).write_bytes(self.payload)

    def test_verified_cache_requires_no_network(self):
        self.archive.write_bytes(self.payload)
        with patch.object(source.subprocess, 'run') as run:
            self.assertEqual(source.fetch_source(self.root, '8.0.1'), self.archive)
            run.assert_not_called()

    def test_corrupt_cache_is_not_replaced(self):
        self.archive.write_bytes(b'bad cache')
        with patch.object(source.subprocess, 'run') as run:
            with self.assertRaises(ValueError):
                source.fetch_source(self.root, '8.0.1')
            run.assert_not_called()
        self.assertEqual(self.archive.read_bytes(), b'bad cache')

    def test_transport_failure_uses_same_pin_and_preserves_partial(self):
        for error in [subprocess.CalledProcessError(28, 'curl'),
                      subprocess.TimeoutExpired('curl', 65)]:
            with self.subTest(error=type(error).__name__):
                def attempt(command, **kwargs):
                    if 'ffmpeg.org/releases' in command[-3]:
                        Path(command[-1]).write_bytes(b'incomplete')
                        raise error
                    self.download(command, **kwargs)
                with patch.object(source.subprocess, 'run', side_effect=attempt) as run:
                    source.fetch_source(self.root, '8.0.1')
                    self.assertEqual(run.call_count, 2)
                self.assertEqual(self.archive.read_bytes(), self.payload)
                self.assertTrue(any(p.read_bytes() == b'incomplete'
                                    for p in self.root.glob('*.partial-*')))
                self.archive.unlink()

    def test_integrity_failure_is_terminal_before_cache_commit(self):
        def corrupt(command, **kwargs):
            Path(command[-1]).write_bytes(b'wrong complete source')
        with patch.object(source.subprocess, 'run', side_effect=corrupt) as run:
            with self.assertRaises(ValueError):
                source.fetch_source(self.root, '8.0.1')
            self.assertEqual(run.call_count, 1)
        self.assertFalse(self.archive.exists())
        self.assertFalse((self.root / 'source.sha256').exists())
        self.assertEqual(len(list(self.root.glob('*.partial-*'))), 1)

    def test_exhausted_transports_fail_without_cache(self):
        with patch.object(source.subprocess, 'run',
                          side_effect=subprocess.CalledProcessError(28, 'curl')) as run:
            with self.assertRaises(subprocess.CalledProcessError):
                source.fetch_source(self.root, '8.0.1')
            self.assertEqual(run.call_count, 2)
        self.assertFalse(self.archive.exists())
        self.assertEqual(len(list(self.root.glob('*.partial-*'))), 2)


if __name__ == '__main__':
    unittest.main()
