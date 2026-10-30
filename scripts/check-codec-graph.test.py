import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("graph", Path(__file__).with_name("check-codec-graph.py"))
graph = importlib.util.module_from_spec(spec)
spec.loader.exec_module(graph)


def fixture():
    names = {"core": "zencodec", "png": "zenpng", "pixels": "zenpixels",
             "old": "zenpixels", "resize": "zenresize", "convert": "zenpixels-convert"}
    deps = {"core": ["pixels"], "png": ["core", "pixels", "convert"],
            "pixels": [], "old": [], "resize": ["old"], "convert": ["pixels"]}
    return {"packages": [{"id": k, "name": v} for k, v in names.items()],
            "resolve": {"nodes": [{"id": k, "dependencies": v} for k, v in deps.items()]}}


class Controls(unittest.TestCase):
    def test_unrelated_legacy_pixels_are_allowed(self):
        self.assertEqual(graph.check(fixture()), (2, []))

    def test_native_split_is_rejected(self):
        data = fixture()
        data["resolve"]["nodes"][1]["dependencies"] = ["core", "old", "convert"]
        self.assertTrue(graph.check(data)[1])

    def test_converter_split_is_rejected(self):
        data = fixture()
        data["resolve"]["nodes"][-1]["dependencies"] = ["old"]
        self.assertTrue(graph.check(data)[1])

    def test_same_version_different_source_is_not_identity(self):
        data = fixture()
        for package in data["packages"]:
            package["version"] = "0.3.1"
        data["resolve"]["nodes"][1]["dependencies"] = ["core", "old"]
        self.assertTrue(graph.check(data)[1])

    def test_missing_resolve_or_interfaces_refuses(self):
        with self.assertRaises(ValueError):
            graph.check({"packages": [], "resolve": None})
        with self.assertRaises(ValueError):
            graph.check({"packages": [], "resolve": {"nodes": []}})


if __name__ == "__main__":
    unittest.main()
