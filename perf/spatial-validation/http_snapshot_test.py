"""Actual child/grandchild cleanup; deadlines do not become graceful skips."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

from http_snapshot import run_command


class CommandLifetime(unittest.TestCase):
    def test_exit_status_is_preserved(self):
        result = run_command([sys.executable, "-c", "raise SystemExit(7)"], os.environ)
        self.assertEqual(result.returncode, 7)

    def test_timeout_stops_the_test_grandchild_too(self):
        # Require the child to start; otherwise this would prove only that an
        # empty command group died. Its delayed write detects leaked execution.
        with tempfile.TemporaryDirectory(dir=Path.home() / "tmp") as directory:
            root = Path(directory)
            grandchild = "import time; from pathlib import Path; time.sleep(2); Path(%r).write_text('leaked')" % str(root / "late")
            parent = "import subprocess,sys,time; from pathlib import Path; p=subprocess.Popen([sys.executable,'-c',%r]); Path(%r).write_text(str(p.pid)); time.sleep(10)" % (grandchild, str(root / "ready"))
            with self.assertRaises(subprocess.TimeoutExpired):
                run_command([sys.executable, "-c", parent], os.environ, timeout=1)
            self.assertTrue((root / "ready").is_file(), "test child never reached the grandchild spawn")
            time.sleep(2)
            self.assertFalse((root / "late").exists(), "grandchild survived the command deadline")


if __name__ == "__main__":
    unittest.main()
