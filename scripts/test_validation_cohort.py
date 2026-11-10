import importlib.util
from pathlib import Path
import re
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
def module(name, filename):
    spec=importlib.util.spec_from_file_location(name,ROOT/'scripts'/filename)
    result=importlib.util.module_from_spec(spec);spec.loader.exec_module(result);return result

class CohortTests(unittest.TestCase):
    def test_checker_detects_drift_repairs_only_shared_sources_and_leaves_vendor_alone(self):
        updater=module('updater','sync-validation-cohort.py')
        with tempfile.TemporaryDirectory(dir=Path.home()/'tmp') as name:
            root=Path(name);(root/'Cargo.toml').write_text((ROOT/'Cargo.toml').read_text())
            old='zenpixels = { git = "https://github.com/imazen/zenpixels", rev = "old" }\n'
            # Use a valid old hex identity; no package resolution is needed here.
            old=old.replace('old','abcdef')
            p=root/'perf/demo/Cargo.toml';p.parent.mkdir(parents=True);p.write_text(old+'other = "1.2.3"\n')
            vendor=root/'perf/demo/vendor/native/Cargo.toml';vendor.parent.mkdir(parents=True);vendor.write_text(old)
            self.assertEqual(updater.sync(root),['perf/demo/Cargo.toml'])
            updater.sync(root,True)
            self.assertEqual(updater.sync(root),[])
            self.assertEqual(vendor.read_text(),old)
            self.assertIn('other = "1.2.3"',p.read_text())
    def test_io_targets_match_the_existing_ci_gate(self):
        runner=module('runner','qualify-native-cohort.py')
        workflow=(ROOT/'.github/workflows/docs.yml').read_text().split('  examples:')[1].split('  jpeg:')[0]
        self.assertEqual(set(re.findall(r'--test (\w+)',workflow)),set(runner.IO_TESTS))
        self.assertEqual(set(re.findall(r'--bin (\w+)',workflow)),set(runner.IO_BINS))

if __name__=='__main__':unittest.main()
