"""Reject the dependency splits that can be hidden by local workspace patches."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "native_preflight", Path(__file__).with_name("check-native-preflight.py"))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class FrameworkGraph(unittest.TestCase):
    def setUp(self):
        self.root = Path(__file__).resolve().parents[2]
        self.metadata = {"packages": [
            {"name": name, "source": None,
             "manifest_path": str(self.root / subdir / "Cargo.toml")}
            for name, subdir in [("zencodec", ""), ("zencodec-io", "zencodec-io"),
                                 ("zencodec-coroutine", "zencodec-coroutine"),
                                 ("zenmeta", "zenmeta")]
        ]}

    def test_one_export_family(self):
        probe.validate_framework_graph(self.metadata, self.root)

    def test_each_missing_companion_refuses(self):
        for index in range(4):
            metadata = copy.deepcopy(self.metadata)
            missing = metadata["packages"].pop(index)
            with self.subTest(name=missing["name"]), self.assertRaisesRegex(ValueError, missing["name"]):
                probe.validate_framework_graph(metadata, self.root)

    def test_registry_or_old_git_duplicate_refuses(self):
        for source in ["registry+https://github.com/rust-lang/crates.io-index",
                       "git+https://github.com/imazen/zencodec?rev=old#old"]:
            for index in range(4):
                metadata = copy.deepcopy(self.metadata)
                duplicate = copy.deepcopy(metadata["packages"][index])
                duplicate["source"] = source
                metadata["packages"].append(duplicate)
                with self.subTest(name=duplicate["name"], source=source), self.assertRaisesRegex(ValueError, "found 2"):
                    probe.validate_framework_graph(metadata, self.root)

    def test_single_foreign_package_is_not_qualified(self):
        for index in range(4):
            for field, value in [("source", "git+https://github.com/imazen/zencodec#other"),
                                 ("manifest_path", str(self.root.parent / "other" / "Cargo.toml"))]:
                metadata = copy.deepcopy(self.metadata)
                metadata["packages"][index][field] = value
                with self.subTest(index=index, field=field), self.assertRaisesRegex(ValueError, "framework export"):
                    probe.validate_framework_graph(metadata, self.root)


if __name__ == "__main__":
    unittest.main()
