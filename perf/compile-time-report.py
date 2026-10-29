#!/usr/bin/env python3
"""Summarize retained cold-build pairs; never rerun or discard observations."""
import argparse
import datetime
import gzip
import hashlib
import json
from pathlib import Path
import re
import statistics
from compile_observations import summarize

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("root", type=Path)
parser.add_argument("log", type=Path)
parser.add_argument("receipt", type=Path)
args = parser.parse_args()
root = args.root.resolve()
packet_path = root / "attempt/packet.json.gz"
with gzip.open(packet_path, "rt") as file:
    packet = json.load(file)
assert packet["complete"] and packet["decision"]["exit_code"] == 0
assert packet["mode"] == "iterate", "this report is explicitly exploratory"
assert len(packet["invocations"]) == 60
plan = packet["plan"]
arms = [plan["baseline"], *plan["candidates"], {"id": "__aa"}]
groups = {}
for invocation in packet["invocations"]:
    assert invocation["status"] == "completed" and invocation["correctness"] == [True]
    directory = Path(invocation["output"])
    build = json.loads((directory / "build.json").read_text())
    # Current runners retain timing evidence outside the disposable target.
    # Historical attempts kept it inside target; both layouts remain readable.
    timings = directory / "cargo-timings"
    if not timings.exists():
        timings = directory / "target/cargo-timings"
    pages = list(timings.glob("cargo-timing-*.html"))
    assert len(pages) == 1
    match = re.search(r"const UNIT_DATA = (\[.*?\]);", pages[0].read_text(), re.S)
    units = [u for u in json.loads(match[1]) if u["name"] == "zencodec"]
    assert len(units) == 1
    key = (plan["cases"][invocation["case"]]["id"], arms[invocation["arm"]]["id"])
    groups.setdefault(key, []).append({"wall": build["wall_seconds"], "unit": units[0]["duration"],
                                      "rss_kib": build["maximum_resident_kib"]})


def stats(values):
    return {"median": statistics.median(values), "min": min(values), "max": max(values)}


measurements = [{"case": case, "arm": arm, "n": len(values),
                 "wall_seconds": stats([v["wall"] for v in values]),
                 "zencodec_unit_seconds": stats([v["unit"] for v in values]),
                 "cargo_peak_rss_kib": stats([v["rss_kib"] for v in values])}
                for (case, arm), values in sorted(groups.items())]
assert all(m["n"] == 5 for m in measurements)
cpu_flags = sum(any(o["cpu_load"] > .05 for o in i["observations"]) for i in packet["invocations"])
heavy_flags = sum(any(o["heavy_process_count"] > 0 for o in i["observations"]) for i in packet["invocations"])
resource_line = [line for line in args.log.read_text().splitlines() if line.startswith("run-heavy: done")]
assert len(resource_line) == 1 and "rc=0" in resource_line[0]
receipt = {"schema": 1, "source_main": plan["candidates"][0]["revision"],
           "source_ideal": plan["candidates"][1]["revision"], "baseline": "crates.io 0.1.26",
           "artifact_directory": str(root), "packet": str(packet_path), "measurements": measurements,
           "quality": {"mode": "iterate", "confirmed": False, "invocations": 60,
                       "cpu_load_flags": cpu_flags, "heavy_process_flags": heavy_flags,
                       "observation_positions": summarize(packet["invocations"]),
                       "placement": packet["placement"]["status"], "correctness_checked": True},
           "scope": "minimal consumer re-export; empty target each invocation; dependencies prefetched; OS file cache warm; two cargo jobs; incremental and compiler wrappers disabled; Rust 1.99.1 on Ryzen 9 7900X; no codec implementations, tests, site generation or CMS",
           "resource_wrapper": resource_line[0], "log": str(args.log),
           "hashes": {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in
                      [packet_path, root / "plan.json", root / "runner-measured.py", root / "toolchain.txt", args.log]}}
args.receipt.write_text(json.dumps(receipt, indent=2) + "\n")
progress = json.loads((root / "progress.json").read_text())
progress.update(checked_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(), stage="completed; exploratory only",
                completed_builds=60, verified_supervisor_pid=None, next_action="publish observed medians and quality limitations")
(root / "progress.json").write_text(json.dumps(progress, indent=2) + "\n")
print(json.dumps({"measurements": measurements, "quality": receipt["quality"]}, indent=2))
