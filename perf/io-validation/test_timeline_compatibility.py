"""The frozen native overlay changes lint annotations, never edit semantics."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('setup_sources', Path(__file__).with_name('setup.py'))
setup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(setup)

class CompatibilityTest(unittest.TestCase):
    def test_only_named_functions_change(self):
        for relative, signatures in setup.TIMELINE_COMPATIBILITY.items():
            source = ('// retained source\n' + '\n'.join(signatures) + '\n').encode()
            actual = setup.timeline_compatibility(relative, source)
            retained = b'\n'.join(line for line in actual.split(b'\n') if b'#[allow(deprecated,' not in line)
            self.assertEqual(retained, source)
            self.assertEqual(actual.count(b'#[allow(deprecated,'), len(signatures))
        self.assertEqual(setup.timeline_compatibility('unrelated.rs', b'unchanged'), b'unchanged')

    def test_source_drift_refuses(self):
        for relative in setup.TIMELINE_COMPATIBILITY:
            with self.assertRaises(RuntimeError): setup.timeline_compatibility(relative, b'changed')

if __name__ == '__main__': unittest.main()
