#!/usr/bin/env python3
"""Resource-observer controls, not compiler throughput measurements.

Ten owned CPU intervals run either in one process or short-lived children.
Both arms are identical. Retain every Confirm outcome, including invalid ones.
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys
import time


def work():
    deadline = time.monotonic() + 0.2
    count = 0
    while time.monotonic() < deadline:
        count += sum(i * i for i in range(1000))
    assert count > 0


def prepare(root):
    root.mkdir(parents=True, exist_ok=False)
    shutil.copyfile(__file__, root / "worker.py")
    arm = {"id": "owned", "program": sys.executable,
           "revision": "owned-process-lifecycle-v1",
           "args": ["${worker}", "run", "${mode}"], "timeout_seconds": 30,
           "env": {"ZENBENCH_LAUNCHER_PIDS": None}}
    candidate = dict(arm, id="identical")
    plan = {"schema": 1, "baseline": arm, "candidates": [candidate],
            "cases": [{"id": mode, "inputs": {"worker": "worker.py", "mode": mode}}
                      for mode in ["resident", "children"]],
            "metric": {"extract": {"kind": "prefix", "prefix": "elapsed_ms="},
                       "unit": "ms", "expected": 1, "resolution": 1, "divide_by": 1},
            "check": {"kind": "json", "prefix": "result=", "pointers": ["/completed"]},
            "warmup": "None; resource-observer control, not throughput evidence.",
            "blocks": 5, "aa": True, "deadline_seconds": 300,
            "confirmation": {"margin_pct": 1, "control_margin_pct": 1,
                             "family_confidence": .95, "scope": "owned-process-lifecycle-v1",
                             "accept": "equivalence"}}
    # Case inputs are files, so put the mode in a frozen small text file.
    for case in plan["cases"]:
        mode = case["id"]
        (root / (mode + ".txt")).write_text(mode)
        case["inputs"]["mode"] = mode + ".txt"
    (root / "plan.json").write_text(json.dumps(plan, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["prepare", "run", "child"])
    parser.add_argument("path", nargs="?", type=Path)
    args = parser.parse_args()
    if args.action == "prepare":
        prepare(args.path)
    elif args.action == "child":
        work()
    else:
        mode = args.path.read_text()
        assert mode in ["resident", "children"]
        start = time.perf_counter()
        completed = 0
        for _ in range(10):
            if mode == "resident":
                work()
            else:
                subprocess.run([sys.executable, __file__, "child"], check=True, timeout=10)
            completed += 1
        print(f"elapsed_ms={(time.perf_counter() - start) * 1000:.6f}")
        print("result=" + json.dumps({"completed": completed}))
