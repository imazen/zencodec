#!/usr/bin/env python3
"""Real-process restoration controls; no existing user process is suspended."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

script = Path(__file__).with_name("quiet-command.py")
scratch = Path.home() / "tmp"
scratch.mkdir(exist_ok=True)
idle = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])


def stopped():
    stat = Path(f"/proc/{idle.pid}/stat").read_text()
    return stat[stat.rindex(")") + 2:].startswith("T")


try:
    with tempfile.TemporaryDirectory(prefix="quiet-command-control-", dir=scratch) as root:
        for label, seconds, code, expected in [
            ("success", 5, "pass", 0),
            ("failure", 5, "raise SystemExit(7)", 7),
            ("timeout", 2.2, "import time; time.sleep(10)", 124),
        ]:
            progress = Path(root) / (label + ".json")
            verify = (f'from pathlib import Path; t=Path("/proc/{idle.pid}/stat").read_text(); '
                      'assert t[t.rindex(")")+2:].startswith("T"); ' + code)
            result = subprocess.run([
                sys.executable, str(script), "--pid", str(idle.pid), "--seconds", str(seconds),
                "--progress", str(progress), "--", sys.executable, "-c", verify,
            ], timeout=8)
            assert result.returncode == expected, (label, result.returncode)
            assert not stopped(), label
            assert json.loads(progress.read_text())["stage"] == "restored"
            print(label, "restored")
        progress = Path(root) / "parent-death.json"
        parent = subprocess.Popen([
            sys.executable, str(script), "--pid", str(idle.pid), "--seconds", "3",
            "--progress", str(progress), "--", sys.executable, "-c", "import time; time.sleep(0.4)",
        ])
        deadline = time.monotonic() + 2
        while not stopped():
            assert time.monotonic() < deadline
            time.sleep(.01)
        parent.kill()
        parent.wait(timeout=2)
        deadline = time.monotonic() + 2
        while stopped() or json.loads(progress.read_text())["stage"] != "watchdog_restored":
            assert time.monotonic() < deadline
            time.sleep(.01)
        print("parent SIGKILL watchdog restored")
finally:
    idle.terminate()
    idle.wait(timeout=2)
