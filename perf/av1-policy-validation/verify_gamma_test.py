"""Negative controls for the independent saved-trace checker."""
import json
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name('verify_gamma.py')


class GammaOracleTests(unittest.TestCase):
    def test_accepts_exact_endpoints_and_rejects_clipping_or_changed_roundtrip(self):
        scratch = Path.home() / 'tmp'
        scratch.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            root = Path(directory)
            trace = root / 'planar-control'
            trace.mkdir()
            (trace / 'gamma-linear-0.raw').write_bytes(struct.pack('<3f', -1, 0, 1))
            for encoded, restored, success in [
                ((-1, 0, 1), (-1, 0, 1), True),
                ((0, 0, 1), (-1, 0, 1), False),
                ((-1, 0, 1), (-0.5, 0, 1), False),
            ]:
                with self.subTest(encoded=encoded, restored=restored):
                    (trace / 'gamma-encoded-0.raw').write_bytes(struct.pack('<3f', *encoded))
                    (trace / 'gamma-restored-0.raw').write_bytes(struct.pack('<3f', *restored))
                    report = root / f'report-{encoded}-{restored}.json'
                    result = subprocess.run([sys.executable, str(SCRIPT), str(root), str(report), '--presentations', '1'], capture_output=True, text=True)
                    self.assertEqual(result.returncode == 0, success, result.stderr)
                    self.assertEqual(report.exists(), success)
                    if success:
                        self.assertEqual(json.loads(report.read_text())['extended_samples'], 1)
            (trace / 'gamma-encoded-0.raw').unlink()
            result = subprocess.run([sys.executable, str(SCRIPT), str(root), str(root / 'missing.json'), '--presentations', '1'], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((root / 'missing.json').exists())


if __name__ == '__main__':
    unittest.main()
