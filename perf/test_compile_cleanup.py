"""The compile runner keeps evidence, not completed build products."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("compile_runner", Path(__file__).with_name("compile-time.py"))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)

class Cleanup(unittest.TestCase):
    def exercise(self, failure=None):
        with tempfile.TemporaryDirectory(dir=Path.home() / "tmp") as scratch:
            root = Path(scratch)
            project = root / "project" / "default"
            project.mkdir(parents=True)
            (project / "Cargo.lock").write_text("frozen lock")
            config = root / "config.json"
            config.write_text(json.dumps({"feature": "default", "profile": "dev"}))
            output = root / "output"
            def fake_run(command, **kwargs):
                target = Path(command[command.index("--target-dir") + 1])
                (target / "cargo-timings").mkdir(parents=True)
                (target / "cargo-timings/cargo-timing-test.html").write_text("timing evidence")
                (target / "regenerable-product").write_text("compiled output")
                (output / "resources.txt").write_text("1.25 1234")
                kwargs["stdout"].write("full compiler log")
                if failure:
                    raise failure
            with patch.object(runner, "run", fake_run), contextlib.redirect_stdout(io.StringIO()):
                if failure:
                    with self.assertRaises(type(failure)):
                        runner.build(root / "project", config, output)
                else:
                    runner.build(root / "project", config, output)
            self.assertEqual((output / "cargo.log").read_text(), "full compiler log")
            if isinstance(failure, subprocess.TimeoutExpired):
                self.assertTrue((output / "target/regenerable-product").exists())
            else:
                self.assertFalse((output / "target").exists())
                self.assertEqual((output / "cargo-timings/cargo-timing-test.html").read_text(), "timing evidence")
            if not failure:
                self.assertEqual(json.loads((output / "build.json").read_text())["maximum_resident_kib"], 1234)

    def test_success(self): self.exercise()
    def test_failure(self): self.exercise(subprocess.CalledProcessError(101, "cargo"))
    def test_timeout_retains_target_until_supervised_exit(self): self.exercise(subprocess.TimeoutExpired("cargo", 120))

if __name__ == "__main__": unittest.main()
