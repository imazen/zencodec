"""Source catalog ownership must not depend on rustdoc index order."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("catalog", Path(__file__).with_name("generate-code-catalog.py"))
catalog = importlib.util.module_from_spec(spec)
spec.loader.exec_module(catalog)


class OwnershipTests(unittest.TestCase):
    def extract(self, order):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "lib.rs").write_text("// fixture\n")
            span = {"filename": "lib.rs", "begin": [1, 0], "end": [1, 10]}
            def item(name, inner):
                return {"name": name, "crate_id": 0, "span": span, "inner": inner}
            index = {
                "1": item("Input", {"struct": {"impls": [3, 5]}}),
                "2": item("Output", {"struct": {"impls": [3, 5]}}),
                "3": item(None, {"impl": {"for": {"resolved_path": {"id": 2}}, "items": [4]}}),
                "4": item("from", {"function": {}}),
                "5": item(None, {"impl": {"for": {"generic": "T"}, "blanket_impl": {"generic": "T"}, "items": [6]}}),
                "6": item("inspect", {"function": {}}),
                "7": item("Inspect", {"trait": {"items": [6]}}),
            }
            data = {"index": {key: index[key] for key in order}, "paths": {
                "1": {"crate_id": 0, "path": ["fixture", "Input"]},
                "2": {"crate_id": 0, "path": ["fixture", "Output"]},
                "7": {"crate_id": 0, "path": ["fixture", "Inspect"]},
            }}
            rustdoc = root / "fixture.json"
            rustdoc.write_text(json.dumps(data))
            return catalog.source_details(rustdoc, root, ".")

    def test_methods_belong_to_implementing_type_and_trait(self):
        first = self.extract(["1", "2", "3", "4", "5", "6", "7"])
        reverse = self.extract(["7", "6", "5", "4", "3", "2", "1"])
        self.assertEqual(first, reverse)
        names = {item["path"] for item in first}
        self.assertIn("fixture::Output::from", names)
        self.assertIn("fixture::Inspect::inspect", names)
        self.assertNotIn("fixture::Input::from", names)
        self.assertNotIn("fixture::Input::inspect", names)
        self.assertNotIn("fixture::Output::inspect", names)




class CacheTests(unittest.TestCase):
    def test_cache_requires_source_tool_and_payload_identity(self):
        import hashlib
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            output = root / 'cache'
            output.mkdir()
            packages = []
            for package, directory in catalog.PACKAGES:
                base = root / directory
                (base / 'src').mkdir(parents=True, exist_ok=True)
                (base / 'Cargo.toml').write_text('[package]\nname="fixture"\n')
                (base / 'src/lib.rs').write_text('pub struct Example;\n')
                (base / 'README.md').write_text('Original documentation\n')
                entry = {'name': package, 'directory': directory,
                         'fingerprint': catalog.fingerprint(root, directory)}
                for suffix, key, data in [('txt', 'surface_sha256', b'pub struct Example\n'),
                                          ('json', 'details_sha256', b'[]\n')]:
                    (output / f'{package}.{suffix}').write_bytes(data)
                    entry[key] = hashlib.sha256(data).hexdigest()
                packages.append(entry)
            identity = {'toolchain': 'frozen', 'generator': 'test'}
            manifest = {'identity': identity, 'packages': packages}
            path = output / 'manifest.json'
            path.write_text(json.dumps(manifest))
            self.assertTrue(catalog.cached_catalog(output, root, identity))
            self.assertFalse(catalog.cached_catalog(output, root, {'toolchain': 'changed'}))
            (root / 'README.md').write_text('Changed docs, identical signatures\n')
            self.assertFalse(catalog.cached_catalog(output, root, identity))
            (root / 'README.md').write_text('Original documentation\n')
            target = output / 'zencodec.json'
            original = target.read_bytes()
            target.write_bytes(b'["corrupt"]')
            self.assertFalse(catalog.cached_catalog(output, root, identity))
            target.write_bytes(original)
            target.unlink()
            self.assertFalse(catalog.cached_catalog(output, root, identity))
            path.write_text('[]')
            self.assertFalse(catalog.cached_catalog(output, root, identity))

if __name__ == "__main__":
    unittest.main()
