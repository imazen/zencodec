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

    def test_consumer_carries_only_pinned_source_patches(self):
        with tempfile.TemporaryDirectory(dir=Path.home() / "tmp") as scratch:
            root = Path(scratch)
            (root / "Cargo.toml").write_text('[patch.crates-io]\nzenpixels = { git = "https://example.test/pixels", rev = "' + 'a' * 40 + '" }\n')
            self.assertIn('rev = "' + 'a' * 40 + '"', runner.root_patches(root))
            (root / "Cargo.toml").write_text('[patch.crates-io]\nzenpixels = { git = "https://example.test/pixels", branch = "main" }\n')
            with self.assertRaises(ValueError):
                runner.root_patches(root)

    def test_report_reads_preserved_timing_evidence(self):
        import gzip
        with tempfile.TemporaryDirectory(dir=Path.home() / "tmp") as scratch:
            root = Path(scratch)
            (root / "attempt").mkdir()
            observations = []
            for case in range(3):
                for arm in range(4):
                    for repeat in range(5):
                        output = root / f"build-{case}-{arm}-{repeat}"
                        (output / "cargo-timings").mkdir(parents=True)
                        (output / "cargo-timings/cargo-timing-test.html").write_text(
                            'const UNIT_DATA = [{"name":"zencodec","duration":1.0}];')
                        (output / "build.json").write_text(json.dumps({
                            "wall_seconds": 2.0, "maximum_resident_kib": 1000}))
                        observations.append({"status": "completed", "correctness": [True],
                            "output": str(output), "case": case, "arm": arm,
                            "observations": [{"cpu_load": 0, "heavy_process_count": 0}]})
            plan = {"baseline": {"id": "published"}, "candidates": [
                {"id": "main", "revision": "before"}, {"id": "ideal", "revision": "after"}],
                "cases": [{"id": str(i)} for i in range(3)]}
            packet = {"complete": True, "decision": {"exit_code": 0}, "mode": "iterate",
                "invocations": observations, "plan": plan, "placement": {"status": "verified"}}
            with gzip.open(root / "attempt/packet.json.gz", "wt") as output:
                json.dump(packet, output)
            for name in ["plan.json", "runner-measured.py", "toolchain.txt"]:
                (root / name).write_text("synthetic test fixture")
            (root / "progress.json").write_text("{}")
            log = root / "run.log"
            log.write_text("run-heavy: done rc=0 synthetic-test\n")
            report = root / "receipt.json"
            result = subprocess.run(["python3", str(Path(__file__).with_name("compile-time-report.py")),
                str(root), str(log), str(report)], capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(len(json.loads(report.read_text())["measurements"]), 12)

    def test_success(self): self.exercise()
    def test_failure(self): self.exercise(subprocess.CalledProcessError(101, "cargo"))
    def test_timeout_retains_target_until_supervised_exit(self): self.exercise(subprocess.TimeoutExpired("cargo", 120))

if __name__ == "__main__": unittest.main()
